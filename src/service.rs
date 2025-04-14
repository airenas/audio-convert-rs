use std::path::Path;

use crate::{
    ffmpeg::wrapper::FFMpegWrapper, otel::make_span, proto_audio_convert::{self, AudioFormat, ConvertInput, ConvertReply}
};
use anyhow::Context;
use async_tempfile::TempDir;
use proto_audio_convert::audio_converter_server::AudioConverter;
use thiserror::Error;
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};
use tonic::{Request, Response, Status};
use tracing::instrument;

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

        self.transcode(input_file, &request.metadata, output_file)
            .await?;

        let res = load_file(output_file).await?;

        let res = ConvertReply { data: res };
        Ok(res)
    }

    #[instrument(skip(metadata))]
    async fn transcode(
        &self,
        input: &str,
        metadata: &Vec<String>,
        output: &str,
    ) -> anyhow::Result<()> {
        tracing::debug!("call ffmpeg");

        let input = input.to_string();
        let metadata = metadata.clone();
        let output = output.to_string();
        let transcoder = self.transcoder.clone();

        tokio::task::spawn_blocking(move || {
            transcoder.transcode(&input, &metadata, &output).context("transcode")
        }).await??;

        tracing::debug!("ffmpeg done");
        Ok(())
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

