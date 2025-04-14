pub mod proto_audio_convert {
    tonic::include_proto!("audio_convert.v1");
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("audio_convert_descriptor");
}

pub mod otel;
pub mod service;
pub mod ffmpeg;

pub const SERVICE_NAME: &str = "audio-convert-rs";


