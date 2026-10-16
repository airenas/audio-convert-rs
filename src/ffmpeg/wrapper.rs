use super::transcoder::transcode;

#[derive(Clone, Debug)]
pub struct FFMpegWrapper;

impl FFMpegWrapper {
    pub fn new() -> anyhow::Result<Self> {
        tracing::debug!("Initializing FFMpegWrapper");
        ffmpeg_the_third::init()?;

        let version = ffmpeg_the_third::util::version();
        let major = version >> 16 & 0xFF;
        let minor = version >> 8 & 0xFF;
        let patch = version & 0xFF;
        tracing::info!("FFMpeg libavutil version: {}.{}.{}", major, minor, patch);
        Ok(FFMpegWrapper {})
    }

    #[tracing::instrument(skip(self, input, metadata, output))]
    pub fn transcode(&self, input: &str, metadata: &[String], output: &str) -> anyhow::Result<()> {
        transcode(input, metadata, output)
    }
}
