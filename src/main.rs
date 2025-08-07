use actix_web::{web::{self, get}, App, HttpResponse, HttpServer, Responder};
use opentelemetry::{
    global,
    trace::{Tracer, TraceContextExt},
    KeyValue,
};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider, metrics::SdkMeterProvider, trace::SdkTracerProvider, Resource,
};
use prometheus::{Encoder, IntCounter, Gauge, Registry, TextEncoder};
use std::{error::Error, sync::OnceLock};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::{prelude::*, EnvFilter};
use sysinfo::{ProcessesToUpdate, System,  get_current_pid};

static RESOURCE: OnceLock<Resource> = OnceLock::new();

fn get_resource() -> Resource {
    RESOURCE
    .get_or_init(|| {
        Resource::builder()
        .with_service_name("otlp-actix-http-example")
        .build()
    })
    .clone()
}


fn init_logs() -> SdkLoggerProvider {
    let exporter = LogExporter::builder()
    .with_http()
    .with_endpoint("http://otel-collector:4318/v1/logs")        .with_protocol(Protocol::HttpBinary)
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
    .with_endpoint("http://otel-collector:4318/v1/traces")
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
    .with_endpoint("http://otel-collector:4318")
    .with_protocol(Protocol::HttpBinary)
    .build()
    .expect("Failed to create metric exporter");
    
    SdkMeterProvider::builder()
    .with_periodic_exporter(exporter)
    .with_resource(get_resource())
    .build()
}

#[derive(Debug)]
struct AppMetrics {
    registry: Registry,
    request_counter: IntCounter,
    memory_gauge: Gauge,
    cpu_gauge: Gauge,
}

impl AppMetrics {
    fn new() -> Self {
        let registry = Registry::new();
        
        let request_counter = IntCounter::new("http_requests_total", "Number of HTTP requests").unwrap();
        let memory_gauge = Gauge::new("app_memory_bytes", "Memory used by the app in bytes").unwrap();
        let cpu_gauge = Gauge::new("app_cpu_percent", "CPU usage percent of the app").unwrap();
        
        registry.register(Box::new(request_counter.clone())).unwrap();
        registry.register(Box::new(memory_gauge.clone())).unwrap();
        registry.register(Box::new(cpu_gauge.clone())).unwrap();
        
        Self {
            registry,
            request_counter,
            memory_gauge,
            cpu_gauge,
        }
    }
}

async fn metrics_handler(data: web::Data<Arc<Mutex<AppMetrics>>>) -> impl Responder {
    let encoder = TextEncoder::new();
    let metrics = data.lock().await;
    let metric_families = metrics.registry.gather();
    
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    
    HttpResponse::Ok()
    .content_type("text/plain; version=0.0.4")
    .body(String::from_utf8(buffer).unwrap())
}

async fn index(metrics: web::Data<Arc<Mutex<AppMetrics>>>) -> impl Responder {
    // Increment request count
    {
        let  metrics = metrics.lock().await;
        metrics.request_counter.inc();
    }
    
    HttpResponse::Ok().body("Hello! This request was counted.")
}

async fn update_system_metrics(metrics: Arc<Mutex<AppMetrics>>) {
    let mut sys = System::new_all();
    let pid = get_current_pid().unwrap().as_u32();
    let get_pid= get_current_pid().unwrap();
    let pid_array= [get_pid];
    
    loop {
        sys.refresh_processes(ProcessesToUpdate::Some(&pid_array), true);
        sys.refresh_cpu_all();
        sys.refresh_memory();
        
        if let Some(proc) = sys.process(sysinfo::Pid::from_u32(pid)) {
            let  metrics = metrics.lock().await;
            metrics.memory_gauge.set(proc.memory() as f64 / 1048576.0); // Bytes → Mb
            metrics.cpu_gauge.set(proc.cpu_usage() as f64);
        }
        
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
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
    .with_filter(EnvFilter::new("info"));
    
    tracing_subscriber::registry()
    .with(otel_layer)
    .with(fmt_layer)
    .init();
    
    let tracer_provider = init_traces();
    global::set_tracer_provider(tracer_provider.clone());
    
    let meter_provider = init_metrics();
    global::set_meter_provider(meter_provider.clone());
    
    let app_metrics = Arc::new(Mutex::new(AppMetrics::new()));
    let metrics_clone = app_metrics.clone();
    tokio::spawn(update_system_metrics(metrics_clone));
    
    let tracer = global::tracer("example");
    tracer.in_span("startup", |cx| {
        let span = cx.span();
        span.set_attribute(KeyValue::new("app.startup", true));
        info!("App is starting...");
    });
    
    info!("Server running at http://0.0.0.0:8888");
    
    HttpServer::new(move || {
        App::new()
        .app_data(web::Data::new(app_metrics.clone()))
        .route("/", web::get().to(index))
        .route("/metrics", web::get().to(metrics_handler))
    })
    .bind(("0.0.0.0", 8888))?
    .run()
    .await?;
    
    tracer_provider.shutdown()?;
    meter_provider.shutdown()?;
    logger_provider.shutdown()?;
    
    Ok(())
}
