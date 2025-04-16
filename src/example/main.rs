use anyhow::{Context, Ok};
use audio_convert_rs::{
    proto_audio_convert::{
        self, audio_converter_client, stream_convert_input::Payload, AudioFormat, InitialMetadata,
        StreamConvertInput,
    },
    SERVICE_NAME,
};
use clap::Parser;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tonic_health::pb::{health_check_response, health_client::HealthClient, HealthCheckRequest};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug, Clone)]
#[command(version = env!("CARGO_APP_VERSION"), name = SERVICE_NAME, about="Client for audio-convert-rs", 
    long_about = None, author="Airenas V.<airenass@gmail.com>")]
struct Args {
    /// GRPC port
    #[arg(short, long, env, default_value = "50051")]
    port: u16,
    /// Input audio file
    #[arg(short = 'i', long, env, default_value = "1.wav")]
    file: String,
    /// Audio output format
    #[arg(short, long, env, default_value = "MP3")]
    format: String,
    /// Stream file
    #[arg(short, long, env, default_value = "false")]
    stream: bool,
    /// Send n times
    #[arg(short, long, env, default_value = "1")]
    times: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::Layer::default().compact())
        .init();
    let args = Args::parse();
    if let Err(e) = main_int(args).await {
        tracing::error!("{:#}", e);
        return Err(e);
    }
    Ok(())
}

async fn main_int(args: Args) -> anyhow::Result<()> {
    tracing::info!(name = SERVICE_NAME, "Starting GRPC client");
    tracing::info!(version = env!("CARGO_APP_VERSION"));
    tracing::info!(grpc_port = args.port);
    tracing::info!(file = args.file);
    tracing::info!(format = args.format);

    check_health(args.port).await?;

    let client =
        audio_converter_client::AudioConverterClient::connect(format!("http://[::]:{}", args.port))
            .await?;
    let audio = std::fs::read(&args.file).with_context(|| format!("read file: {}", args.file))?;

    for _i in 0..args.times {
        transcode(args.clone(), client.clone(), audio.clone()).await?;
    }
    Ok(())
}

async fn transcode(
    args: Args,
    client: audio_converter_client::AudioConverterClient<Channel>,
    audio: Vec<u8>,
) -> anyhow::Result<()> {
    let data = if args.stream {
        tracing::info!("streaming");
        stream_file(client, audio, &args.format)
            .await
            .with_context(|| format!("stream file: {}", args.file))?
    } else {
        tracing::info!("simple sync call");
        simple_call(client, audio, &args.format)
            .await
            .with_context(|| format!("simple call file: {}", args.file))?
    };

    let output_file = format!("{}.{}", args.file, args.format.to_ascii_lowercase());
    tracing::info!(output_file, "saving...");
    let mut file = tokio::fs::File::create(output_file)
        .await
        .with_context(|| "failed to create file")?;
    file.write_all(&data)
        .await
        .with_context(|| format!("failed to write file: {:?}", "output.mp3"))?;

    Ok(())
}

async fn stream_file(
    mut client: audio_converter_client::AudioConverterClient<Channel>,
    audio: Vec<u8>,
    format: &str,
) -> anyhow::Result<Vec<u8>> {
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    let input_stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    let format_cp = format.to_owned();
    tokio::spawn(async move {
        let metadata = StreamConvertInput {
            payload: Some(Payload::Metadata(InitialMetadata {
                format: AudioFormat::from_str_name(&format_cp).unwrap_or_default() as i32,
                metadata: vec!["test=olia".to_string(), "copyright=smth...".to_string()],
            })),
        };
        if tx.send(metadata).await.is_err() {
            tracing::error!("Failed to send metadata");
            return;
        }

        // Send the audio file in chunks
        let mut offset = 0;
        let chunk_size = 64 * 1024; // 64 KB
        while offset < audio.len() {
            let end = (offset + chunk_size).min(audio.len());
            let chunk = StreamConvertInput {
                payload: Some(Payload::Chunk(audio[offset..end].to_vec())),
            };
            if tx.send(chunk).await.is_err() {
                tracing::error!("Failed to send audio chunk");
                return;
            }
            offset = end;
        }
    });

    let mut request = tonic::Request::new(input_stream);
    request.metadata_mut().insert(
        "traceparent",
        tonic::metadata::MetadataValue::try_from(
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
        )
        .unwrap(),
    );
    let mut response_stream = client.convert_stream(request).await?.into_inner();

    let mut converted_audio = Vec::new();
    while let Some(response) = response_stream.next().await {
        let response = response?;
        converted_audio.extend(response.chunk);
    }

    Ok(converted_audio)
}

async fn simple_call(
    mut client: audio_converter_client::AudioConverterClient<Channel>,
    audio: Vec<u8>,
    format: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut request = tonic::Request::new(proto_audio_convert::ConvertInput {
        format: AudioFormat::from_str_name(format).unwrap_or_default() as i32,
        metadata: vec!["test=olia".to_string(), "copyright=smth...".to_string()],
        data: audio,
    });
    tracing::info!(message = "Sending request");
    request.metadata_mut().insert(
        "traceparent",
        tonic::metadata::MetadataValue::try_from(
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
        )
        .unwrap(),
    );
    let result = client.convert(request).await?;

    let data = result.get_ref().clone();
    Ok(data.data)
}

async fn check_health(port: u16) -> anyhow::Result<()> {
    let channel = Channel::from_shared(format!("http://[::]:{}", port))?
        .connect()
        .await?;
    let mut client = HealthClient::new(channel.clone());
    let health_request = tonic::Request::new(HealthCheckRequest {
        service: "".to_string(),
    });

    let result = client.check(health_request).await?;
    let response = result.get_ref();
    tracing::info!(status = response.status, "got a response.");
    if response.status != health_check_response::ServingStatus::Serving as i32 {
        return Err(anyhow::anyhow!("Service is not healthy"));
    }
    Ok(())
}
