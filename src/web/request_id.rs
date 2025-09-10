use axum::body::Body;
use axum::http::{HeaderName, Request};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{HttpMakeClassifier, TraceLayer};
use tracing::{error, info_span, Span};

const REQUEST_ID_HEADER: &str = "x-request-id";
static X_REQUEST_ID: HeaderName = HeaderName::from_static(REQUEST_ID_HEADER);

pub fn set_request_id_layer() -> SetRequestIdLayer<MakeRequestUuid>{
    SetRequestIdLayer::new(X_REQUEST_ID.clone(), MakeRequestUuid)
}
// Fuck rust, how do I simplify the return type?
pub fn trace_layer() -> TraceLayer<HttpMakeClassifier, fn(&Request<Body>) -> Span> {
    TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
        // Log the request id as generated.
        let request_id = request.headers().get(REQUEST_ID_HEADER);
        match request_id {
            Some(request_id) => info_span!(
                        "http_request",
                        request_id = ?request_id,
                    ),
            None => {
                error!("could not extract request_id");
                info_span!("http_request")
            }
        }
    })
}
pub fn propagate_request_id_layer() -> PropagateRequestIdLayer {
    PropagateRequestIdLayer::new(X_REQUEST_ID.clone())
}
