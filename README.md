# prom_otel

This Rust application instruments traces, metrics, and logs using OpenTelemetry and exposes HTTP endpoints via Hyper.

## Prerequisites

- Docker & Docker Compose
- Kubernetes cluster (e.g., Minikube, Kind)
- `kubectl` configured

## Local Setup

1. Build and run with Docker Compose:
   ```bash
   docker-compose up --build
   ```
2. Verify services:
   - Health check:  
     ```bash
     curl http://localhost:3000/healthz
     ```
     Should return `OK`.
   - Root endpoint:  
     ```bash
     curl http://localhost:3000/
     ```
     Should return `Hello, OpenTelemetry!`.
   - Metrics placeholder:  
     ```bash
     curl http://localhost:3000/metrics
     ```

3. Observe logs & metrics in OTLP collector container logs:
   ```bash
   docker-compose logs otel-collector
   ```

## Docker

- `Dockerfile` builds the Rust binary and runs it in a slim Debian container.
- Environment variables:
  - `SERVER_ADDR` (default `0.0.0.0:3000`)
  - `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://otel-collector:4317`)
  - `RUST_LOG` (default `info`)

## Kubernetes Deployment

1. Apply manifests:
   ```bash
   kubectl apply -f k8s/manifests.yaml
   ```
2. Port-forward service:
   ```bash
   kubectl -n observability port-forward svc/prom-otel-service 3000:80
   ```
3. Test endpoints:
   ```bash
   curl http://localhost:3000/healthz
   curl http://localhost:3000/
   curl http://localhost:3000/metrics
   ```
4. View collector logs:
   ```bash
   kubectl -n observability logs deploy/otel-collector
   ```

## Verification

- Traces and metrics appear in the collector logs.
- Logs from the application are also visible.
