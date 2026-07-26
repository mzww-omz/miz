pub mod api;
mod application;
pub mod authorization;
mod federation;
mod infrastructure;
mod observability;
pub mod security;

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    routing::get,
};
use security::SecurityState;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = infrastructure::database(&database_url)
        .await
        .expect("database connection and migrations must succeed");
    let state = SecurityState {
        pool,
        origin: std::env::var("APP_ORIGIN").expect("APP_ORIGIN must be set"),
    };
    let app = app(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("API port must be available");
    axum::serve(listener, app).await.expect("API server failed");
}

fn app(state: SecurityState) -> Router {
    let protected_api =
        Router::new()
            .fallback(api::not_found)
            .layer(middleware::from_fn_with_state(
                state.clone(),
                security::require_session,
            ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(ready))
        .route("/openapi.json", get(api::openapi))
        .route("/metrics", get(observability::metrics))
        .nest("/api/v1", protected_api)
        .fallback(api::not_found)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(
                    observability::REQUEST_ID_HEADER,
                    observability::MakeRequestIdFromCSPRNG,
                ))
                .layer(PropagateRequestIdLayer::new(
                    observability::REQUEST_ID_HEADER,
                ))
                .layer(TraceLayer::new_for_http())
                .layer(middleware::from_fn(observability::count_requests)),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state)
}

async fn ready(State(state): State<SecurityState>) -> StatusCode {
    match state.pool.acquire().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use sqlx::PgPool;
    use tower::ServiceExt;

    #[tokio::test]
    async fn api_is_deny_by_default() {
        let pool = PgPool::connect_lazy("postgres://miz:miz@localhost/miz").unwrap();
        let response = app(SecurityState {
            pool,
            origin: "https://m1z.jp".to_owned(),
        })
        .oneshot(
            Request::get("/api/v1/unlisted")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let request_id = response.headers()[observability::REQUEST_ID_HEADER]
            .to_str()
            .unwrap();
        assert_eq!(request_id.len(), 22);
    }
}
