use std::env;
use std::time::Duration;

use opentelemetry::Context;
use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::runtime;
use opentelemetry_sdk::trace;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_stdout::SpanExporter;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::SERVICE_NAME;

pub struct TracerGuard;

impl Drop for TracerGuard {
    fn drop(&mut self) {
        tracing::info!("shutting down tracer");
        opentelemetry::global::shutdown_tracer_provider();
    }
}

pub fn init_tracer() -> anyhow::Result<(opentelemetry_sdk::trace::TracerProvider, String)> {
    let service_name = env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| SERVICE_NAME.to_string());
    let otel_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    let sampling_rate: f64 = env::var("OTEL_SAMPLING_RATE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);

    global::set_text_map_propagator(TraceContextPropagator::new());

    let info: String;

    let tracer_provider = match otel_endpoint.as_deref() {
        Some("") | None => {
            info = "No OTLP endpoint".to_owned();
            opentelemetry_sdk::trace::TracerProvider::builder()
                .with_config(trace::Config::default())
                .build()
        }
        Some("stdout") => {
            info = "OTLP stdout exporter".to_owned();
            TracerProvider::builder()
                .with_simple_exporter(SpanExporter::default())
                .build()
        }
        Some(endpoint) => {
            info = format!("OTLP endpoint: {}", endpoint);

            let exporter = opentelemetry_otlp::new_exporter()
                .http()
                .with_endpoint(endpoint)
                .with_timeout(Duration::from_secs(5));

            opentelemetry_otlp::new_pipeline()
                .tracing()
                .with_exporter(exporter)
                .with_trace_config(
                    opentelemetry_sdk::trace::Config::default()
                        .with_sampler(opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(
                            sampling_rate,
                        ))
                        .with_resource(opentelemetry_sdk::Resource::new(vec![KeyValue::new(
                            "service.name",
                            service_name,
                        )])),
                )
                .install_batch(runtime::Tokio)?
        }
    };

    global::set_tracer_provider(tracer_provider.clone());

    Ok((tracer_provider, info))
}

pub fn make_span(headers: &http::HeaderMap) -> tracing::Span {
    let cx = extract_context_from_request(headers);
    let trace_id = cx.span().span_context().trace_id().to_string();
    let res = tracing::info_span!("request", otel.kind = "server", trace_id,);
    res.set_parent(cx);
    res
}

fn extract_context_from_request(headers: &http::HeaderMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)))
}
