use ffmpeg_the_third::{self as ffmpeg, Dictionary};

use std::env;
use std::path::Path;

use ffmpeg::codec::Parameters;
use ffmpeg::{codec, filter, frame, media};

fn filter(
    spec: &str,
    decoder: &codec::decoder::Audio,
    encoder: &codec::encoder::Audio,
) -> Result<filter::Graph, ffmpeg::Error> {
    let mut filter = filter::Graph::new();

    let channel_layout = format!("0x{:x}", 2);

    let args = format!(
        "time_base={}:sample_rate={}:sample_fmt={}:channel_layout={channel_layout}",
        decoder.time_base(),
        decoder.rate(),
        decoder.format().name()
    );

    filter.add(&filter::find("abuffer").unwrap(), "in", &args)?;
    filter.add(&filter::find("abuffersink").unwrap(), "out", "")?;

    {
        let mut out = filter.get("out").unwrap();
        out.set_sample_format(encoder.format());
        out.set_ch_layout(encoder.ch_layout());
        // out.set_sample_rate(encoder.rate());
    }

    filter.output("in", 0)?.input("out", 0)?.parse(spec)?;
    filter.validate()?;

    // println!("{}", filter.dump());

    if let Some(codec) = encoder.codec() {
        if !codec
            .capabilities()
            .contains(ffmpeg::codec::capabilities::Capabilities::VARIABLE_FRAME_SIZE)
        {
            tracing::info!(size = encoder.frame_size(), "Setting frame size");
            filter
                .get("out")
                .unwrap()
                .sink()
                .set_frame_size(encoder.frame_size());
        }
    }

    Ok(filter)
}

struct Transcoder {
    stream: usize,
    filter: filter::Graph,
    decoder: codec::decoder::Audio,
    encoder: codec::encoder::Audio,
    in_time_base: ffmpeg::Rational,
    out_time_base: ffmpeg::Rational,
}

fn transcoder<P: AsRef<Path> + ?Sized>(
    ictx: &mut ffmpeg::format::context::Input,
    octx: &mut ffmpeg::format::context::Output,
    path: &P,
    filter_spec: &str,
) -> Result<Transcoder, ffmpeg::Error> {
    let input = ictx
        .streams()
        .best(media::Type::Audio)
        .expect("could not find best audio stream");
    let context = ffmpeg::codec::context::Context::from_parameters(input.parameters())?;
    let mut decoder = context.decoder().audio()?;
    let codec = ffmpeg::encoder::find(octx.format().codec(path, media::Type::Audio))
        .expect("failed to find encoder")
        .audio()
        .expect("encoder is not audio encoder");

    tracing::info!(name = codec.description(), "codec");
    let global = octx
        .format()
        .flags()
        .contains(ffmpeg::format::flag::Flags::GLOBAL_HEADER);

    decoder.set_parameters(input.parameters())?;

    let mut output = octx.add_stream(codec)?;
    let context = ffmpeg::codec::context::Context::from_parameters(output.parameters())?;
    let mut encoder = context.encoder().audio()?;

    if global {
        encoder.set_flags(ffmpeg::codec::flag::Flags::GLOBAL_HEADER);
    }

    let channel_layout = codec
        .ch_layouts()
        .map(|cls| cls.best(decoder.ch_layout().channels()))
        .unwrap_or(ffmpeg::channel_layout::ChannelLayout::MONO);
    tracing::info!(channel_layout = ?channel_layout, "Configured channel layout");
    encoder.set_ch_layout(channel_layout);
    tracing::info!(
        channels = decoder.ch_layout().channels(),
        "Configured channels"
    );
    encoder.set_rate(decoder.rate() as i32);
    tracing::info!(rate = decoder.rate(), "Configured rate");
    encoder.set_format(
        codec
            .formats()
            .expect("unknown supported formats")
            .next()
            .unwrap(),
    );
    // tracing::info!(rate = decoder.bit_rate(), "Configured bit rate");
    encoder.set_max_bit_rate(decoder.max_bit_rate());
    if codec.description().contains("mp3") {
        tracing::info!(bit_rate = 50_000, "Configured bit rate for mp3");
        encoder.set_quality(4);
        encoder.set_bit_rate(50_000);
    }
    // tracing::info!(max_bit_rate = decoder.max_bit_rate(), "Configured max bit rate");

    encoder.set_time_base((1, decoder.rate() as i32));
    output.set_time_base((1, decoder.rate() as i32));

    let encoder = encoder.open_as(codec)?;
    output.set_parameters(Parameters::from(&encoder));

    let filter = filter(filter_spec, &decoder, &encoder)?;

    let in_time_base = decoder.time_base();
    let out_time_base = output.time_base();

    Ok(Transcoder {
        stream: input.index(),
        filter,
        decoder,
        encoder,
        in_time_base,
        out_time_base,
    })
}

impl Transcoder {
    fn send_frame_to_encoder(&mut self, frame: &ffmpeg::Frame) -> anyhow::Result<()> {
        self.encoder.send_frame(frame)?;
        Ok(())
    }

    fn send_eof_to_encoder(&mut self) -> anyhow::Result<()> {
        self.encoder.send_eof()?;
        Ok(())
    }

    fn receive_and_process_encoded_packets(
        &mut self,
        octx: &mut ffmpeg::format::context::Output,
    ) -> anyhow::Result<()> {
        let mut encoded = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(0);
            encoded.rescale_ts(self.in_time_base, self.out_time_base);
            encoded.write_interleaved(octx)?;
        }
        Ok(())
    }

    fn add_frame_to_filter(&mut self, frame: &ffmpeg::Frame) -> anyhow::Result<()> {
        self.filter.get("in").unwrap().source().add(frame)?;
        Ok(())
    }

    fn flush_filter(&mut self) -> anyhow::Result<()> {
        self.filter.get("in").unwrap().source().flush()?;
        Ok(())
    }

    fn get_and_process_filtered_frames(
        &mut self,
        octx: &mut ffmpeg::format::context::Output,
    ) -> anyhow::Result<()> {
        let mut filtered = frame::Audio::empty();
        while self
            .filter
            .get("out")
            .ok_or_else(|| anyhow::anyhow!("Failed to get 'out' filter sink"))?
            .sink()
            .frame(&mut filtered)
            .is_ok()
        {
            self.send_frame_to_encoder(&filtered)?;
            self.receive_and_process_encoded_packets(octx)?;
            drop(filtered); 
            filtered = frame::Audio::empty();
        }
        Ok(())
    }

    fn send_packet_to_decoder(&mut self, packet: &ffmpeg::Packet) -> anyhow::Result<()> {
        self.decoder.send_packet(packet)?;
        Ok(())
    }

    fn send_eof_to_decoder(&mut self) -> anyhow::Result<()> {
        self.decoder.send_eof()?;
        Ok(())
    }

    fn receive_and_process_decoded_frames(
        &mut self,
        octx: &mut ffmpeg::format::context::Output,
    ) -> anyhow::Result<()> {
        let mut decoded = frame::Audio::empty();
        while self.decoder.receive_frame(&mut decoded).is_ok() {
            let timestamp = decoded.timestamp();
            decoded.set_pts(timestamp);
            self.add_frame_to_filter(&decoded)?;
            self.get_and_process_filtered_frames(octx)?;
        }
        Ok(())
    }
}

#[tracing::instrument(skip(metadata))]
pub fn transcode(input: &str, metadata: &[String], output: &str) -> anyhow::Result<()> {
    let filter = env::args().nth(3).unwrap_or_else(|| "anull".to_owned());
    let mut ictx = ffmpeg::format::input(input)?;
    let mut octx = ffmpeg::format::output(output)?;
    let mut transcoder = transcoder(&mut ictx, &mut octx, output, &filter)?;

    let combined_metadata = prepare_metadata(ictx.metadata().to_owned(), metadata)?;
    octx.set_metadata(combined_metadata);
    octx.write_header()?;

    for (stream, mut packet) in ictx.packets().filter_map(Result::ok) {
        if stream.index() == transcoder.stream {
            packet.rescale_ts(stream.time_base(), transcoder.in_time_base);
            transcoder.send_packet_to_decoder(&packet)?;
            transcoder.receive_and_process_decoded_frames(&mut octx)?;
        }
    }

    transcoder.send_eof_to_decoder()?;
    transcoder.receive_and_process_decoded_frames(&mut octx)?;

    transcoder.flush_filter()?;
    transcoder.get_and_process_filtered_frames(&mut octx)?;

    transcoder.send_eof_to_encoder()?;
    transcoder.receive_and_process_encoded_packets(&mut octx)?;

    octx.write_trailer()?;

    tracing::trace!("Transcoding completed successfully");
    tracing::trace!("Cleaning up resources");
    drop(transcoder);
    drop(octx);
    drop(ictx);

    Ok(())
}

fn prepare_metadata<'a>(
    metadata: Dictionary<'a>,
    metadata_add: &[String],
) -> anyhow::Result<Dictionary<'a>> {
    let mut combined_metadata = metadata;

    // Add additional metadata to the dictionary
    for s in metadata_add.iter() {
        let mut parts = s.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        if key.is_empty() {
            return Err(anyhow::anyhow!("Invalid metadata key-value pair"));
        }
        combined_metadata.set(key, value);
    }
    Ok(combined_metadata)
}
