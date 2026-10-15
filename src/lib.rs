#[allow(clippy::result_large_err, clippy::double_must_use)]
pub mod proto_audio_convert {
    tonic::include_proto!("audio_convert.v1");
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("audio_convert_descriptor");
}

pub mod ffmpeg;
pub mod otel;
pub mod service;
pub mod ulaw;

pub const SERVICE_NAME: &str = "audio-convert-rs";
