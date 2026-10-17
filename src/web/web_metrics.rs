use std::future::ready;
use std::time::Instant;
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::Router;
use axum::routing::get;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use tokio::net::ToSocketAddrs;
use tokio_util::sync::CancellationToken;
use tracing::info;
use crate::service::content_check;

pub async fn start_metrics_server(
    addr: impl ToSocketAddrs,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    let app = metrics_app();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Metrics server listening on {}", listener.local_addr()?);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            cancel_token.cancelled().await;
            info!("Shutting down metrics server...");
        })
        .await?;
    Ok(())
}

fn metrics_app() -> Router {
    let recorder_handle = setup_metrics_recorder();
    Router::new().route("/metrics", get(move || ready(recorder_handle.render())))
}

fn setup_metrics_recorder() -> PrometheusHandle {
    prometheus_builder().install_recorder().unwrap()
}

fn prometheus_builder() -> PrometheusBuilder {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    // Histograms without buckets are exported as summaries, which can't be combined across the
    // instances that run side by side during a rollout. Give each new histogram its buckets here.
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_requests_duration_seconds".to_string()),
            EXPONENTIAL_SECONDS,
        )
        .unwrap()
        .set_buckets_for_metric(
            Matcher::Full(content_check::METRIC_DURATION.to_string()),
            content_check::DURATION_BUCKETS,
        )
        .unwrap()
}

pub async fn track_metrics(req: Request, next: Next) -> impl IntoResponse {
    let start = Instant::now();
    let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
        matched_path.as_str().to_owned()
    } else {
        req.uri().path().to_owned()
    };
    let method = req.method().clone();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    let labels = [
        ("method", method.to_string()),
        ("path", path),
        ("status", status),
    ];

    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_requests_duration_seconds", &labels).record(latency);

    response
}

#[cfg(test)]
mod tests {
    use super::prometheus_builder;
    use crate::service::content_check;
    use std::time::Duration;

    #[test]
    fn exports_content_checks_with_buckets() {
        let recorder = prometheus_builder().build_recorder();
        metrics::with_local_recorder(&recorder, || {
            content_check::record_metrics("user.bio", false, &["sexual".to_string()], Some(540), Duration::from_millis(400));
            content_check::record_metrics("user.bio", true, &[], None, Duration::from_secs(3));
            content_check::record_metrics("user.username", false, &[], Some(530), Duration::from_millis(80));
        });
        let out = recorder.handle().render();

        assert!(out.contains(r#"content_check_total{subject="user.bio",outcome="block"} 1"#), "{out}");
        assert!(out.contains(r#"content_check_total{subject="user.bio",outcome="error"} 1"#), "{out}");
        assert!(out.contains(r#"content_check_total{subject="user.username",outcome="pass"} 1"#), "{out}");
        assert!(out.contains(r#"content_check_blocked_total{subject="user.bio",question="sexual"} 1"#), "{out}");
        assert!(out.contains(r#"content_check_input_tokens_total{subject="user.bio"} 540"#), "{out}");
        // A histogram, not a summary: the 400 ms check falls in the 0.5 s bucket
        assert!(out.contains(r#"content_check_duration_seconds_bucket{subject="user.bio",outcome="block",le="0.5"} 1"#), "{out}");
        assert!(out.contains(r#"content_check_duration_seconds_bucket{subject="user.bio",outcome="block",le="0.25"} 0"#), "{out}");
        assert!(!out.contains("content_check_duration_seconds{"), "{out}");
    }
}
