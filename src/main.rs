use audio_convert_rs::{
    SERVICE_NAME, ffmpeg, otel,
    proto_audio_convert::{self, audio_converter_server::AudioConverterServer},
    service::Service, ulaw,
};
use clap::Parser;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tonic::transport::Server;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(version = env!("CARGO_APP_VERSION"), name = SERVICE_NAME, about="Service for audio convertion WAV -> MP3, M4A", 
    long_about = None, author="Airenas V.<airenass@gmail.com>")]
struct Args {
    /// GRPC server port
    #[arg(long, env, default_value = "50051")]
    port: u16,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let _guard = otel::TracerGuard;

    use opentelemetry::trace::TracerProvider as _;

    let (provider, tr_info) = otel::init_tracer()?;
    let tracer = provider.tracer(SERVICE_NAME);
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::Layer::default().compact())
        .with(telemetry)
        .init();
    tracing::info!(info = tr_info, "tracer");
    let args = Args::parse();
    if let Err(e) = main_int(args).await {
        tracing::error!("{}", e);
        return Err(e);
    }
    Ok(())
}

async fn main_int(args: Args) -> anyhow::Result<()> {
    tracing::info!(name = SERVICE_NAME, "Starting GRPC service");
    tracing::info!(version = env!("CARGO_APP_VERSION"));
    tracing::info!(port = args.port);

    let cancel_token = CancellationToken::new();

    let ct = cancel_token.clone();

    tokio::spawn(async move {
        let mut int_stream = signal(SignalKind::interrupt()).unwrap();
        let mut term_stream = signal(SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = int_stream.recv() => tracing::info!("Exit event int"),
            _ = term_stream.recv() => tracing::info!("Exit event term"),
            // _ = rx_exit_indicator.recv() => log::info!("Exit event from some loader"),
        }
        tracing::debug!("sending exit event");
        ct.cancel();
        tracing::debug!("expected drop tx_close");
    });

    let address: std::net::SocketAddr = format!("[::]:{}", args.port).parse()?;
    tracing::info!(address = format!("{:?}", address), "address");

    let ffmpeg_wrapper = ffmpeg::wrapper::FFMpegWrapper::new()?;
    let ulaw_transcoder = ulaw::transcoder::ULaw::new()?;
    let service = Service::new(ffmpeg_wrapper, ulaw_transcoder);
    let grpc_service = AudioConverterServer::new(service);
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto_audio_convert::FILE_DESCRIPTOR_SET)
        .build_v1alpha()?;
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<AudioConverterServer<Service>>()
        .await;

    let ct = cancel_token.clone();
    let grpc_server = Server::builder()
        .add_service(reflection_service)
        .add_service(health_service)
        .add_service(grpc_service)
        .serve_with_shutdown(address, async move {
            ct.cancelled().await;
        });

    grpc_server.await?;

    tracing::info!("Service stopped");
    Ok(())
}
