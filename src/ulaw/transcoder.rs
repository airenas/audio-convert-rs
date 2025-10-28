use std::{
    fs::File,
    io::{BufWriter, Write},
};

use anyhow::Context;
use codec_core::codecs::g711::ulaw_compress;
use rubato::{FftFixedInOut, Resampler};

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
            tracing::debug!(len = input[0].len(), "Resampling chunk");
            let (in_size, out_size) = resampler
                .process_into_buffer(&input, &mut output_buf, None)
                .context("Resampling failed")?;
            let out_chunk = &output_buf[0][0..out_size];
            tracing::debug!(
                input_len = len,
                output_len = out_chunk.len(),
                in_size,
                out_size,
                "Resampled chunk"
            );
            resampled.extend_from_slice(out_chunk);
        }

        tracing::debug!(len = resampled.len(), "Resampled");

        let expected_samples = (8000.0 * (samples.len() as f32 / spec.sample_rate as f32)).round() as usize;
        
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
}

fn linear_to_mulaw(sample: f32) -> u8 {
    let pcm = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
    ulaw_compress(pcm)
}

