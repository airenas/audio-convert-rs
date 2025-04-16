use std::path::Path;

use crate::{
    ffmpeg::wrapper::FFMpegWrapper,
    otel::make_span,
    proto_audio_convert::{
        self, AudioFormat, ConvertInput, ConvertReply, StreamConvertInput, StreamFileReply,
    },
};
use anyhow::Context;
use async_tempfile::TempDir;
use proto_audio_convert::audio_converter_server::AudioConverter;
use thiserror::Error;
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWriteExt},
};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{instrument, Instrument};

#[derive(Debug)]
pub struct Service {
    transcoder: FFMpegWrapper,
}

impl Service {
    pub fn new(transcoder: FFMpegWrapper) -> Self {
        tracing::info!("new service");
        Self { transcoder }
    }

    #[instrument(skip(request))]
    async fn convert(&self, request: &ConvertInput) -> Result<ConvertReply, SrvError> {
        tracing::info!(len = request.data.len(), "got a request");

        let audio_format = AudioFormat::try_from(request.format).map_err(|_| {
            SrvError::InvalidArgument(format!("invalid audio format: {}", request.format))
        })?;
        validate_format(audio_format)?;

        let dir = TempDir::new().await.with_context(|| "tmp dir create")?;
        let dir_str = dir.to_str().context("no tmp dir path")?;

        let input_path = Path::new(dir_str).join("input.wav");
        let input_file = input_path.to_str().unwrap();
        save_audio(input_file, &request.data).await?;

        let output_path = Path::new(dir_str).join(prepare_output_file(audio_format)?);
        let output_file = output_path.to_str().unwrap();

        transcode(
            self.transcoder.clone(),
            input_file,
            &request.metadata,
            output_file,
        )
        .await?;

        let res = load_file(output_file).await?;

        let res = ConvertReply { data: res };
        Ok(res)
    }
}

fn prepare_output_file(format: AudioFormat) -> anyhow::Result<String> {
    let ext = match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::M4a => "m4a",
        AudioFormat::Unspecified => unreachable!(),
    };
    Ok(format!("output.{}", ext))
}

#[tonic::async_trait]
impl AudioConverter for Service {
    async fn convert(
        &self,
        request: Request<ConvertInput>, // Stream of ConvertRequest from client
    ) -> Result<Response<ConvertReply>, Status> {
        tracing::trace!(metadata= ?request.metadata(), "Received requests");

        let span = make_span(request.metadata().as_ref());
        let _enter = span.enter();

        let req = request.get_ref();
        tracing::info!(
            metadata = req.metadata.len(),
            len = req.data.len(),
            format = req.format,
            "input"
        );
        let res = self.convert(request.get_ref()).await;
        match res {
            Ok(r) => Ok(Response::new(r)),
            Err(e) => {
                tracing::error!(error = ?e, "convert error");
                Err(e.into())
            }
        }
    }

    type ConvertStreamStream = ReceiverStream<Result<StreamFileReply, Status>>;

    async fn convert_stream(
        &self,
        request: tonic::Request<tonic::Streaming<StreamConvertInput>>,
    ) -> Result<tonic::Response<Self::ConvertStreamStream>, tonic::Status> {
        tracing::trace!(metadata= ?request.metadata(), "Received requests");
        let span = make_span(request.metadata().as_ref());

        let mut input_stream = request.into_inner();

        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let transcoder = self.transcoder.clone();
        tokio::spawn(
            async move {
                if let Err(e) =
                    async {
                        let dir = TempDir::new().await.with_context(|| "tmp dir create")?;
                        let dir_str = dir.to_str().context("no tmp dir path")?;

                        let mut metadata: Vec<String> = Vec::new();
                        let input_path = Path::new(dir_str).join("input.wav");
                        let input_file_name = input_path.to_str().unwrap();

                        let mut output_file_name: Option<String> = None;

                        {
                            let save_span = tracing::info_span!("saving file");
                            let _enter = save_span.enter(); // Enter the span
                            let mut input_file =
                                File::create(input_file_name).await.context("file create")?;
                            while let Some(Ok(input)) = input_stream.next().await {
                                match input.payload {
                            Some(proto_audio_convert::stream_convert_input::Payload::Metadata(
                                meta,
                            )) => {
                                tracing::trace!("Received header: {:?}", meta);
                                let audio_format =
                                    AudioFormat::try_from(meta.format).map_err(|_| {
                                        SrvError::InvalidArgument(format!(
                                            "invalid audio format: {}",
                                            meta.format
                                        ))
                                    })?;
                                tracing::info!(format = audio_format.as_str_name());
                                validate_format(audio_format)?;
                                let output_path =
                                    Path::new(dir_str).join(prepare_output_file(audio_format)?);
                                output_file_name = Some(output_path.to_str().unwrap().to_string());
                                metadata = meta.metadata.clone();
                            }
                            Some(proto_audio_convert::stream_convert_input::Payload::Chunk(
                                chunk,
                            )) => {
                                tracing::trace!(
                                    file = input_file_name,
                                    len = chunk.len(),
                                    "writing"
                                );
                                input_file.write_all(&chunk).await.context("write file")?
                            }
                            None => {
                                return Err(SrvError::InvalidArgument(
                                    "No payload in StreamConvertInput".to_string(),
                                ));
                            }
                        }
                            }
                        }
                        tracing::debug!("Saved file");
                        match output_file_name {
                            Some(name) => {
                                transcode(transcoder, input_file_name, &metadata, &name).await?;
                                {
                                    let write_span = tracing::info_span!("sending result");
                                    let _enter = write_span.enter();

                                    tracing::debug!("Sending result");
                                    let mut output_reader = tokio::fs::File::open(&name)
                                        .await
                                        .context(format!("can't open {}", name))?;
                                    let mut buffer = vec![0u8; 1024 * 64];
                                    loop {
                                        match output_reader.read(&mut buffer).await {
                                            Ok(0) => break,
                                            Ok(n) => {
                                                let reply = StreamFileReply {
                                                    chunk: buffer[..n].to_vec(),
                                                };
                                                tracing::trace!(
                                                    file = input_file_name,
                                                    len = n,
                                                    "sending"
                                                );
                                                tx.send(Ok(reply)).await.context("can't send")?;
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to read output file: {}",
                                                    e
                                                );
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            None => {
                                return Err(SrvError::InvalidArgument(
                                    "No format provided".to_string(),
                                ));
                            }
                        }
                        tracing::debug!("Done");
                        Ok(())
                    }
                    .await
                {
                    tracing::error!("Error in convert_stream: {}", e);
                    let _ = tx
                        .send(Err(Status::internal(format!("Error: {}", e))))
                        .await;
                }
            }
            .instrument(span.clone()),
        );

        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }
}

fn validate_format(format: AudioFormat) -> Result<(), SrvError> {
    if format == AudioFormat::Unspecified {
        return Err(SrvError::InvalidArgument(
            "audio format unspecified".to_string(),
        ));
    }
    Ok(())
}

#[instrument(skip(audio))]
async fn save_audio(file: &str, audio: &[u8]) -> anyhow::Result<()> {
    tracing::debug!(file, "writing file");
    let mut file = File::create(file).await.context("file create")?;
    file.write_all(audio).await.context("file write")?;
    Ok(())
}

#[instrument()]
async fn load_file(file: &str) -> anyhow::Result<Vec<u8>> {
    tracing::debug!(file, "reading file");
    let content = fs::read(file)
        .await
        .with_context(|| format!("failed to read file: {:?}", file))?;

    Ok(content)
}

#[derive(Debug, Error)]
pub enum SrvError {
    #[error("{0}")]
    InvalidArgument(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<SrvError> for Status {
    fn from(err: SrvError) -> Self {
        match err {
            SrvError::InvalidArgument(msg) => Status::invalid_argument(msg),
            SrvError::Other(e) => Status::internal(format!("error: {}", e)),
        }
    }
}

#[instrument(skip(metadata))]
async fn transcode(
    transcoder: FFMpegWrapper,
    input: &str,
    metadata: &[String],
    output: &str,
) -> anyhow::Result<()> {
    tracing::debug!("call ffmpeg");
    let input = input.to_string();
    let metadata = metadata.to_owned();
    let output = output.to_string();

    let span = tracing::info_span!("spawn_blocking_transcode");
    tokio::task::spawn_blocking(move || {
        let _enter = span.enter();
        transcoder
            .transcode(&input, &metadata, &output)
            .context("transcode")
    })
    .await??;

    tracing::debug!("ffmpeg done");
    Ok(())
}
