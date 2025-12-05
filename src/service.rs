use std::{future::Future, path::Path, pin::Pin};

use crate::{
    ffmpeg::wrapper::FFMpegWrapper,
    otel::make_span,
    proto_audio_convert::{
        self, AudioFormat, ConvertInput, ConvertReply, StreamConvertInput, StreamFileReply,
    },
    ulaw,
};
use anyhow::Context;
use async_tempfile::TempDir;
use futures::ready;
use proto_audio_convert::audio_converter_server::AudioConverter;
use std::task::{Context as TaskContext, Poll};
use thiserror::Error;
use tokio::io::{Error as IoError, ErrorKind};
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc::Sender,
};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{instrument, Instrument};

#[derive(Debug)]
pub struct Service {
    transcoder: FFMpegWrapper,
    ulaw_transcoder: ulaw::transcoder::ULaw,
}

impl Service {
    pub fn new(transcoder: FFMpegWrapper, ulaw_transcoder: ulaw::transcoder::ULaw) -> Self {
        tracing::info!("new ulaw service");
        Self {
            transcoder,
            ulaw_transcoder,
        }
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

        if audio_format == AudioFormat::Ulaw {
            tracing::debug!("call ulaw stream transcode");
            let file = File::create(output_file).await.context("file create")?;
            let writer = tokio::io::BufWriter::new(file);
            stream_transcode_ulaw(self.ulaw_transcoder.clone(), input_file, writer).await?;
        } else {
            transcode(
                self.transcoder.clone(),
                input_file,
                &request.metadata,
                output_file,
            )
            .await?;
        }

        let res = load_file(output_file).await?;

        let res = ConvertReply { data: res };
        Ok(res)
    }
}

fn prepare_output_file(format: AudioFormat) -> anyhow::Result<String> {
    let ext = match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::M4a => "m4a",
        AudioFormat::Ulaw => "ulaw",
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

        let req = request.get_ref();
        let fut = async {
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
        };
        fut.instrument(span).await
    }

    type ConvertStreamStream = ReceiverStream<Result<StreamFileReply, Status>>;

    async fn convert_stream(
        &self,
        request: tonic::Request<tonic::Streaming<StreamConvertInput>>,
    ) -> Result<tonic::Response<Self::ConvertStreamStream>, tonic::Status> {
        tracing::trace!(metadata= ?request.metadata(), "Received requests");
        let span = make_span(request.metadata().as_ref());

        let mut input_stream = request.into_inner();

        let meta = input_stream.next().await.ok_or_else(|| {
            SrvError::InvalidArgument("Stream ended before metadata".to_string())
        })??;
        let (audio_format, metadata) =
            if let Some(proto_audio_convert::stream_convert_input::Payload::Metadata(meta)) =
                meta.payload
            {
                tracing::trace!("Received header: {:?}", meta);
                let audio_format = AudioFormat::try_from(meta.format).map_err(|_| {
                    SrvError::InvalidArgument(format!("invalid audio format: {}", meta.format))
                })?;
                tracing::info!(format = audio_format.as_str_name());
                validate_format(audio_format)?;
                let metadata = meta.metadata.clone();
                (audio_format, metadata)
            } else {
                return Err(SrvError::InvalidArgument(
                    "First message must be metadata".to_string(),
                )
                .into());
            };

        let (tx, rx) = tokio::sync::mpsc::channel(10);

        if audio_format == AudioFormat::Ulaw {
            tracing::info!("Using ulaw transcoding");
            let ulaw_transcoder = self.ulaw_transcoder.clone();
            let writer = AsyncChannelWriter::new(tx.clone(), 64 * 1024);
            tokio::spawn(
                async move {
                    if let Err(e) = async {
                        let dir = TempDir::new().await.with_context(|| "tmp dir create")?;
                        let dir_str = dir.to_str().context("no tmp dir path")?;

                        let input_path = Path::new(dir_str).join("input.wav");
                        let input_file_name = input_path.to_str().unwrap();

                        let save_span = tracing::info_span!("saving file");
                        async {
                            let mut input_file =
                                File::create(input_file_name).await.context("file create")?;
                            while let Some(Ok(input)) = input_stream.next().await {
                                if let Some(
                                    proto_audio_convert::stream_convert_input::Payload::Chunk(
                                        chunk,
                                    ),
                                ) = input.payload
                                {
                                    tracing::trace!(
                                        file = input_file_name,
                                        len = chunk.len(),
                                        "writing"
                                    );
                                    input_file.write_all(&chunk).await.context("write file")?;
                                } else {
                                    return Err(SrvError::InvalidArgument(
                                        "Invalid payload in StreamConvertInput".to_string(),
                                    ));
                                }
                            }
                            input_file.sync_all().await.context("sync file")?;
                            tracing::trace!("Finished saving input file");
                            Ok::<(), SrvError>(())
                        }
                        .instrument(save_span)
                        .await?;

                        tracing::debug!("Saved file");

                        stream_transcode_ulaw(ulaw_transcoder, input_file_name, writer).await?;
                        tracing::debug!("Done");
                        Ok::<(), anyhow::Error>(())
                    }
                    .await
                    {
                        tracing::error!("Error in convert_stream: {:?}", e);
                        let _ = tx
                            .send(Err(Status::internal(format!("Error: {}", e))))
                            .await;
                    }
                }
                .instrument(span),
            );
        } else {
            tracing::info!("Using ffmpeg transcoding");
            let transcoder = self.transcoder.clone();
            tokio::spawn(
                async move {
                    if let Err(e) = async {
                        let dir = TempDir::new().await.with_context(|| "tmp dir create")?;
                        let dir_str = dir.to_str().context("no tmp dir path")?;

                        let input_path = Path::new(dir_str).join("input.wav");
                        let input_file_name = input_path.to_str().unwrap();

                        let output_file_name =
                            Path::new(dir_str).join(prepare_output_file(audio_format)?);

                        let save_span = tracing::info_span!("saving file");
                        async {
                            let mut input_file =
                                File::create(input_file_name).await.context("file create")?;
                            while let Some(Ok(input)) = input_stream.next().await {
                                if let Some(
                                    proto_audio_convert::stream_convert_input::Payload::Chunk(
                                        chunk,
                                    ),
                                ) = input.payload
                                {
                                    tracing::trace!(
                                        file = input_file_name,
                                        len = chunk.len(),
                                        "writing"
                                    );
                                    input_file.write_all(&chunk).await.context("write file")?;
                                } else {
                                    return Err(SrvError::InvalidArgument(
                                        "Invalid payload in StreamConvertInput".to_string(),
                                    ));
                                }
                            }
                            input_file.sync_all().await.context("sync file")?;
                            tracing::trace!("Finished saving input file");
                            Ok::<(), SrvError>(())
                        }
                        .instrument(save_span)
                        .await?;

                        tracing::debug!("Saved file");
                        transcode(transcoder, input_file_name, &metadata, &output_file_name)
                            .await?;

                        let write_span = tracing::info_span!("sending result");
                        async {
                            tracing::debug!("Sending result");
                            let mut output_reader = tokio::fs::File::open(&output_file_name)
                                .await
                                .context(format!("can't open {:?}", output_file_name))?;
                            let mut buffer = vec![0u8; 1024 * 64];
                            loop {
                                match output_reader.read(&mut buffer).await {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        let reply = StreamFileReply {
                                            chunk: buffer[..n].to_vec(),
                                        };
                                        tracing::trace!(file = input_file_name, len = n, "sending");
                                        tx.send(Ok(reply)).await.context("can't send")?;
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to read output file: {}", e);
                                        break;
                                    }
                                }
                            }
                            Ok::<(), anyhow::Error>(())
                        }
                        .instrument(write_span)
                        .await?;
                        tracing::debug!("Done");
                        Ok::<(), anyhow::Error>(())
                    }
                    .await
                    {
                        tracing::error!("Error in convert_stream: {:?}", e);
                        let _ = tx
                            .send(Err(Status::internal(format!("Error: {}", e))))
                            .await;
                    }
                }
                .instrument(span),
            );
        }

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
async fn transcode<P: AsRef<std::path::Path> + Clone + std::fmt::Debug>(
    transcoder: FFMpegWrapper,
    input: &str,
    metadata: &[String],
    output: P,
) -> anyhow::Result<()> {
    tracing::debug!("call ffmpeg");
    let input = input.to_string();
    let metadata = metadata.to_owned();
    let output = output.as_ref().to_string_lossy().to_string();

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

#[instrument(skip(writer))]
async fn stream_transcode_ulaw<W: AsyncWrite + Unpin + Send>(
    transcoder: ulaw::transcoder::ULaw,
    input: &str,
    writer: W,
) -> anyhow::Result<()> {
    tracing::debug!("call ulaw transcode");
    transcoder
        .stream_transcode(input, writer)
        .await
        .context("transcode")?;

    tracing::debug!("ulaw done");
    Ok(())
}

pub struct AsyncChannelWriter {
    tx: Sender<Result<StreamFileReply, tonic::Status>>,
    buffer: Vec<u8>,
    chunk_size: usize,
    sending: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    closed: bool,
}

impl AsyncChannelWriter {
    pub fn new(tx: Sender<Result<StreamFileReply, tonic::Status>>, chunk_size: usize) -> Self {
        Self {
            tx,
            buffer: Vec::with_capacity(chunk_size),
            chunk_size,
            sending: None,
            closed: false,
        }
    }

    fn poll_send_next_chunk(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), IoError>> {
        if let Some(mut fut) = self.sending.take() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    self.sending = Some(fut);
                    return Poll::Pending;
                }
                Poll::Ready(()) => {}
            }
        }

        if self.buffer.len() >= self.chunk_size {
            let chunk = self.buffer.drain(..self.chunk_size).collect::<Vec<u8>>();
            let tx = self.tx.clone();
            let fut = Box::pin(async move {
                let _ = tx.send(Ok(StreamFileReply { chunk })).await;
            });
            self.sending = Some(fut);
            return self.poll_send_next_chunk(cx);
        }

        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for AsyncChannelWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
        if self.closed {
            return Poll::Ready(Err(IoError::new(ErrorKind::BrokenPipe, "writer is closed")));
        }
        ready!(self.as_mut().poll_send_next_chunk(cx))?;

        self.buffer.extend_from_slice(buf);

        let _ = self.as_mut().poll_send_next_chunk(cx);

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Result<(), IoError>> {
        ready!(self.as_mut().poll_send_next_chunk(cx))?;

        if !self.buffer.is_empty() && self.sending.is_none() {
            let chunk = std::mem::take(&mut self.buffer);
            let tx = self.tx.clone();
            let fut = Box::pin(async move {
                let _ = tx.send(Ok(StreamFileReply { chunk })).await;
            });
            self.sending = Some(fut);
            return self.as_mut().poll_send_next_chunk(cx);
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<Result<(), IoError>> {
        self.closed = true;
        Poll::Ready(Ok(()))
    }
}
