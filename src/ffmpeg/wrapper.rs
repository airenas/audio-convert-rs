use super::transcoder::transcode;

#[derive(Clone, Debug)]
pub struct FFMpegWrapper;

impl FFMpegWrapper {
    pub fn new() -> anyhow::Result<Self> {
        tracing::debug!("Initializing FFMpegWrapper");
        ffmpeg_the_third::init()?;
        tracing::debug!("FFMpegWrapper initialized");
        Ok(FFMpegWrapper {})
    }

    #[tracing::instrument(skip(self, input, metadata, output))]
    pub fn transcode(
        &self,
        input: &str,
        metadata: &[String],
        output: &str,
    ) -> anyhow::Result<()> {
        transcode(input, metadata, output)
    }
}
