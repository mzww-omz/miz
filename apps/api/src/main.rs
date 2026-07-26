mod api;
mod application;
mod federation;
mod infrastructure;

use axum::{Router, extract::State, http::StatusCode, routing::get};
use sqlx::PgPool;

#[tokio::main]
async fn main() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = infrastructure::database(&database_url)
        .await
        .expect("database connection and migrations must succeed");
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(ready))
        .route("/openapi.json", get(api::openapi))
        .fallback(api::not_found)
        .with_state(pool);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("API port must be available");
    axum::serve(listener, app).await.expect("API server failed");
}

async fn ready(State(pool): State<PgPool>) -> StatusCode {
    match pool.acquire().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
