use axum::{
    extract::Request,
    http::{HeaderValue, header::HeaderName},
    middleware::Next,
    response::Response,
};
use miz_api::domain::RequestId as DomainRequestId;
use std::sync::atomic::{AtomicU64, Ordering};
use tower_http::request_id::{MakeRequestId, RequestId};

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
static REQUESTS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct MakeRequestIdFromCSPRNG;

impl MakeRequestId for MakeRequestIdFromCSPRNG {
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        DomainRequestId::new()
            .ok()
            .and_then(|id| HeaderValue::from_str(&id.to_string()).ok())
            .map(RequestId::new)
    }
}

pub async fn count_requests(request: Request, next: Next) -> Response {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    next.run(request).await
}

pub async fn metrics() -> String {
    format!(
        "# TYPE miz_http_requests_total counter\nmiz_http_requests_total {}\n",
        REQUESTS.load(Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_opaque_public_ids() {
        let request = axum::http::Request::new(());
        let id = MakeRequestIdFromCSPRNG.make_request_id(&request).unwrap();
        let value = id.header_value().to_str().unwrap();
        assert_eq!(value.len(), 22);
        assert!(value.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }
}
