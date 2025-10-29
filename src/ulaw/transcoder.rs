use std::fs;
use std::io;

use anyhow::Context;
use codec_core::codecs::g711::ulaw_compress;
use hound::WavReader;
use rubato::{FftFixedInOut, Resampler};
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

const I16_MAX: f32 = 32767.0;

#[derive(Clone, Debug)]
pub struct ULaw;

impl ULaw {
    pub fn new() -> anyhow::Result<Self> {
        tracing::debug!("Initializing ULaw");
        Ok(Self {})
    }

    // input wav file path, output ulaw writer
    #[tracing::instrument(skip(self, input, writer))]
    pub async fn stream_transcode<W: AsyncWrite + Unpin + Send>(
        &self,
        input: &str,
        mut writer: W,
    ) -> anyhow::Result<()> {
        tracing::info!(%input, "Converting WAV -> µ-law 8kHz");

        let mut reader = hound::WavReader::open(input)
            .with_context(|| format!("Cannot open input wav: {}", input))?;
        let spec = reader.spec();
        tracing::debug!(rate = spec.sample_rate, "Input loaded");
        let channels = spec.channels as usize;
        if channels != 1 {
            return Err(anyhow::anyhow!(
                "Only mono WAV files are supported, found {} channels",
                channels
            ));
        }

        let mut resampler = FftFixedInOut::<f32>::new(spec.sample_rate as usize, 8000, 1024, 1)
            .context("Failed to create resampler")?;

        let chunk_size = resampler.input_frames_next();
        let mut input_buf = vec![0.0f32; chunk_size];
        let mut output_buf = vec![vec![0.0f32; resampler.output_frames_max()]; 1];

        loop {
            let filled = fill_buffer(&mut reader, &mut input_buf, chunk_size)?;
            if filled == 0 {
                break;
            }
            let input = [&input_buf[..]];
            tracing::trace!(len = input[0].len(), "Resampling chunk");
            let (in_size, out_size) = resampler
                .process_into_buffer(&input, &mut output_buf, None)
                .context("Resampling failed")?;
            let out_chunk = &output_buf[0][0..out_size];
            tracing::trace!(
                output_len = out_chunk.len(),
                in_size,
                out_size,
                "Resampled chunk"
            );
            let wanted_samples =
                (8000.0 * (filled as f32 / spec.sample_rate as f32)).ceil() as usize;
            if wanted_samples > out_chunk.len() {
                tracing::warn!(
                    wanted_samples,
                    out_chunk_len = out_chunk.len(),
                    "Resampler produced less samples than expected"
                );
            }
            let out_chunk = &out_chunk[0..wanted_samples];
            for s in out_chunk {
                let ulaw_byte = linear_to_mulaw(*s);
                writer
                    .write_all(&[ulaw_byte])
                    .await
                    .context("Failed to write ulaw data")?;
            }
        }
        writer.flush().await.context("Failed to flush ulaw data")?;

        tracing::info!("µ-law conversion complete");
        Ok(())
    }
}

fn fill_buffer(
    reader: &mut WavReader<io::BufReader<fs::File>>,
    input_buf: &mut [f32],
    chunk_size: usize,
) -> anyhow::Result<usize> {
    let samples = reader.samples::<i16>().take(chunk_size);
    let mut res = 0;
    for sample in samples.map(|s| s.unwrap() as f32 / (I16_MAX + 1.0)) {
        input_buf[res] = sample;
        res += 1;
    }
    if res == 0 {
        return Ok(0);
    }
    for item in input_buf.iter_mut().take(chunk_size).skip(res) {
        *item = 0.0;
    }
    Ok(res)
}

fn linear_to_mulaw(sample: f32) -> u8 {
    let pcm = (sample * I16_MAX).clamp(-I16_MAX - 1.0, I16_MAX) as i16;
    ulaw_compress(pcm)
}
