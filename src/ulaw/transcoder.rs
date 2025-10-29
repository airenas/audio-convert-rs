use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
};

use anyhow::Context;
use codec_core::codecs::g711::ulaw_compress;
use hound::WavReader;
use rubato::{FftFixedInOut, Resampler};
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

#[derive(Clone, Debug)]
pub struct ULaw;

impl ULaw {
    pub fn new() -> anyhow::Result<Self> {
        tracing::debug!("Initializing ULaw");
        Ok(Self {})
    }

    // input wav file path, output ulaw file path
    #[tracing::instrument(skip(self, input, output))]
    pub fn transcode(&self, input: &str, output: &str) -> anyhow::Result<()> {
        tracing::info!(%input, %output, "Converting WAV -> µ-law 8kHz");

        // --- 1. Read input WAV ---
        let mut reader = hound::WavReader::open(input)
            .with_context(|| format!("Cannot open input wav: {}", input))?;
        let spec = reader.spec();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();

        tracing::debug!(rate = spec.sample_rate, "Input loaded");

        // --- 2. Resample to 8kHz ---
        let channels = spec.channels as usize;
        if channels != 1 {
            return Err(anyhow::anyhow!(
                "Only mono WAV files are supported, found {} channels",
                channels
            ));
        }

        tracing::debug!(len = samples.len(), "Input");

        let mut resampler = FftFixedInOut::<f32>::new(spec.sample_rate as usize, 8000, 1024, 1)
            .context("Failed to create resampler")?;

        let chunk_size = resampler.input_frames_next();
        let mut input_buf = vec![0.0f32; chunk_size];
        let mut output_buf = vec![vec![0.0f32; resampler.output_frames_max()]; 1];
        let mut resampled: Vec<f32> = Vec::new();
        for start in (0..samples.len()).step_by(chunk_size) {
            let end = (start + chunk_size).min(samples.len());
            let len = end - start;
            input_buf[..len].copy_from_slice(&samples[start..end]);
            if len < chunk_size {
                input_buf[len..].fill(0.0);
            }
            let input = [&input_buf[..]];
            tracing::trace!(len = input[0].len(), "Resampling chunk");
            let (in_size, out_size) = resampler
                .process_into_buffer(&input, &mut output_buf, None)
                .context("Resampling failed")?;
            let out_chunk = &output_buf[0][0..out_size];
            tracing::trace!(
                input_len = len,
                output_len = out_chunk.len(),
                in_size,
                out_size,
                "Resampled chunk"
            );
            resampled.extend_from_slice(out_chunk);
        }

        tracing::debug!(len = resampled.len(), "Resampled");

        let expected_samples =
            (8000.0 * (samples.len() as f32 / spec.sample_rate as f32)).round() as usize;

        let file = File::create(output).context("Failed to create output file")?;
        let mut writer = BufWriter::new(file);

        for s in &resampled[0..expected_samples] {
            let ulaw_byte = linear_to_mulaw(*s);
            writer
                .write_all(&[ulaw_byte])
                .context("Failed to write ulaw data")?;
        }
        writer.flush().context("Failed to flush ulaw data")?;

        tracing::info!("µ-law conversion complete");
        Ok(())
    }

    // input wav file path, output ulaw file path
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
            let wanted_samples = (8000.0 * (filled as f32 / spec.sample_rate as f32)).ceil() as usize;
            if wanted_samples < out_chunk.len() {
                tracing::warn!(
                    wanted_samples,
                    out_chunk_len = out_chunk.len(),
                    "Resampler produced more samples than expected"
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
    for sample in samples.map(|s| s.unwrap() as f32 / 32768.0) {
        input_buf[res] = sample;
        res += 1;
    }
    if res == 0 {
        return Ok(0);
    }
    for i in res..chunk_size {
        input_buf[i] = 0.0;
    }
    Ok(res)
}

fn linear_to_mulaw(sample: f32) -> u8 {
    let pcm = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
    ulaw_compress(pcm)
}
