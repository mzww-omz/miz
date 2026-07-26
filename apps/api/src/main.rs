mod application;
mod domain;
mod federation;
mod infrastructure;

use axum::{Router, routing::get};

#[tokio::main]
async fn main() {
    let app = Router::new().route("/healthz", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
