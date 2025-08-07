use actix_web::{web, App, HttpServer, Responder};
use opentelemetry::{
    global,
    trace::{TraceContextExt, Tracer},
    KeyValue,
};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::SdkMeterProvider,
    trace::SdkTracerProvider,
    Resource,
};
use prometheus::{Encoder, IntCounter, Registry, TextEncoder};
use std::{error::Error, sync::OnceLock};
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::{prelude::*, EnvFilter};

use std::sync::Arc;

static METRICS: OnceLock<Arc<Mutex<AppMetrics>>> = OnceLock::new();

fn get_resource() -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| {
            Resource::builder()
                .with_service_name("otlp-actix-http-example")
                .build()
        })
        .clone()
}

// === Initialization Functions ===

fn init_logs() -> SdkLoggerProvider {
    let exporter = LogExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .build()
        .expect("Failed to create log exporter");

    SdkLoggerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(get_resource())
        .build()
}

fn init_traces() -> SdkTracerProvider {
    let exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .build()
        .expect("Failed to create trace exporter");

    SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(get_resource())
        .build()
}

fn init_metrics() -> SdkMeterProvider {
    let exporter = MetricExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .build()
        .expect("Failed to create metric exporter");

    SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(get_resource())
        .build()
}

// === Prometheus Metrics for /metrics endpoint ===

#[derive(Debug)]
struct AppMetrics {
    registry: Registry,
    test_counter: IntCounter,
}

impl AppMetrics {
    fn new() -> Self {
        let registry = Registry::new();
        let test_counter = IntCounter::new("test_counter", "A simple counter").unwrap();
        registry.register(Box::new(test_counter.clone())).unwrap();

        Self {
            registry,
            test_counter,
        }
    }
}

async fn metrics_handler(data: web::Data<Arc<Mutex<AppMetrics>>>) -> impl Responder {
    let encoder = TextEncoder::new();
    let metrics = data.lock().await;
    let metric_families = metrics.registry.gather();

    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    String::from_utf8(buffer).unwrap()
}

// === Main Function ===

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    // === Logs ===
    let logger_provider = init_logs();
    let otel_layer = OpenTelemetryTracingBridge::new(&logger_provider);
    let otel_layer = otel_layer.with_filter(
        EnvFilter::new("info")
            .add_directive("hyper=off".parse().unwrap())
            .add_directive("tonic=off".parse().unwrap())
            .add_directive("h2=off".parse().unwrap())
            .add_directive("reqwest=off".parse().unwrap()),
    );

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_filter(EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap()));

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer)
        .init();

    // === Traces and Metrics ===
    let tracer_provider = init_traces();
    global::set_tracer_provider(tracer_provider.clone());

    let meter_provider = init_metrics();
    global::set_meter_provider(meter_provider.clone());

    // === Prometheus Counter ===
    let app_metrics = Arc::new(Mutex::new(AppMetrics::new()));
    METRICS.set(app_metrics.clone()).unwrap();

    let app_metrics_clone = app_metrics.clone();
    tokio::spawn(async move {
        loop {
            {
                let mut metrics = app_metrics_clone.lock().await;
                metrics.test_counter.inc();
                info!("Incremented Prometheus counter");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    // === Sample Trace ===
    let tracer = global::tracer("example");
    tracer.in_span("Main operation", |cx| {
        let span = cx.span();
        span.set_attribute(KeyValue::new("example.key", "value"));
        info!("This is inside a traced span!");
    });

    // === Actix Server ===
    info!("Starting Actix-web server on http://0.0.0.0:4318/metrics");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(app_metrics.clone()))
            .route("/metrics", web::get().to(metrics_handler))
    })
    .bind(("0.0.0.0", 8888))?
    .run()
    .await?;

    // === Shutdown ===
    if let Err(e) = tracer_provider.shutdown() {
        eprintln!("Tracer shutdown error: {e}");
    }

    if let Err(e) = meter_provider.shutdown() {
        eprintln!("Meter shutdown error: {e}");
    }

    if let Err(e) = logger_provider.shutdown() {
        eprintln!("Logger shutdown error: {e}");
    }

    Ok(())
}
