use axum::http::{HeaderName, Request};
use tower::layer::util::{Identity, Stack};
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{HttpMakeClassifier, TraceLayer};
use tracing::{error, info_span, Span};

const REQUEST_ID_HEADER: &str = "x-request-id";
static X_REQUEST_ID: HeaderName = HeaderName::from_static(REQUEST_ID_HEADER);

pub fn request_id_layer<T>() -> ServiceBuilder<Stack<PropagateRequestIdLayer, Stack<TraceLayer<HttpMakeClassifier, fn(&Request<T>) -> Span>, Stack<SetRequestIdLayer<MakeRequestUuid>, Identity>>>> {
    ServiceBuilder::new()
        .layer(set_request_id_layer())
        .layer(trace_layer())
        // send headers from request to response headers
        .layer(propagate_request_id_layer())
}

pub fn set_request_id_layer() -> SetRequestIdLayer<MakeRequestUuid>{
    SetRequestIdLayer::new(X_REQUEST_ID.clone(), MakeRequestUuid)
}
// Fuck rust, how do I simplify the return type?
pub fn trace_layer<T>() -> TraceLayer<HttpMakeClassifier, fn(&Request<T>) -> Span> {
    TraceLayer::new_for_http().make_span_with(|request: &Request<T>| {
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
