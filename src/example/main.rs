use std::time::Duration;

use anyhow::{Context, Ok};
use audio_convert_rs::{
    proto_audio_convert::{self, audio_converter_client, AudioFormat},
    SERVICE_NAME,
};
use clap::Parser;
use tokio::io::AsyncWriteExt;
use tokio_retry::{
    strategy::{jitter, ExponentialBackoff},
    RetryIf,
};
use tonic::{transport::Channel, Code};
use tonic_health::pb::{health_check_response, health_client::HealthClient, HealthCheckRequest};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
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
    /// Dry run
    #[arg(short, long, env, default_value = "false")]
    dry_run: bool,
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

    tracing::info!(len = audio.len(), message = "Sending request",);

    let retry_strategy = ExponentialBackoff::from_millis(100)
        .max_delay(Duration::from_secs(20))
        .factor(2)
        .take(3)
        .map(jitter);

    let mut retry = 0;
    let response = RetryIf::spawn(
        retry_strategy,
        || {
            retry += 1;
            let mut client = client.clone();
            let mut request = tonic::Request::new(proto_audio_convert::ConvertInput {
                format: AudioFormat::from_str_name(&args.format).unwrap_or_default() as i32,
                metadata: vec!["test=olia".to_string(), "copyright=smth...".to_string()],
                data: audio.clone(),
            });
            async move {
                tracing::info!(message = "Sending request", retry = retry);
                request.metadata_mut().insert(
                    "traceparent",
                    tonic::metadata::MetadataValue::try_from(
                        "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01",
                    )
                    .unwrap(),
                );
                let result = if args.dry_run {
                    tracing::info!(message = "dry run");
                    client.convert_dry_run(request).await
                } else {
                    client.convert(request).await
                };

                match &result {
                    Err(status) if status.code() == Code::InvalidArgument => {
                        tracing::error!("Invalid file type: {}", status.message());
                        return result;
                    }
                    _ => {}
                }
                result
            }
        },
        |err: &tonic::Status| err.code() != Code::InvalidArgument,
    )
    .await
    .with_context(|| "failed to transcribe audio after multiple retries")?;

    let resp = response.get_ref();
    tracing::info!(message = "got a response.", len = resp.data.len());

    let output_file = format!("{}.{}", args.file, args.format.to_ascii_lowercase());
    tracing::info!(output_file, "saving...");
    let mut file = tokio::fs::File::create(output_file)
        .await
        .with_context(|| "failed to create file")?;
    file.write_all(&resp.data)
        .await
        .with_context(|| format!("failed to write file: {:?}", "output.mp3"))?;

    Ok(())
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
