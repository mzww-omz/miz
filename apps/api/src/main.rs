pub mod api;
mod application;
pub mod authorization;
mod federation;
mod observability;
mod posts;
mod profile;
pub mod security;
mod sessions;

use axum::{
    Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware,
    routing::get,
};
use miz_api::infrastructure;
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
    let protected_api = Router::new()
        .route(
            "/users/me",
            get(profile::get_current_user).patch(profile::update_current_user),
        )
        .route("/posts", axum::routing::post(posts::create_post))
        .route(
            "/posts/{postId}",
            get(posts::get_post)
                .patch(posts::update_post)
                .delete(posts::delete_post),
        )
        .route(
            "/sessions",
            get(sessions::list_sessions).delete(sessions::revoke_all_sessions),
        )
        .route(
            "/sessions/current",
            axum::routing::delete(sessions::revoke_current_session),
        )
        .route(
            "/sessions/{sessionId}",
            axum::routing::delete(sessions::revoke_session),
        )
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

    #[tokio::test]
    async fn authenticated_post_creation_uses_the_http_contract() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = PgPool::connect(&database_url).await.unwrap();
        infrastructure::migrate(&pool).await.unwrap();
        let user_id = miz_api::domain::UserId::new().unwrap();
        let session_id = miz_api::domain::SessionId::new().unwrap();
        let tokens = security::SessionTokens::generate().unwrap();
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'HTTP Author')")
            .bind(user_id.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, idle_expires_at, absolute_expires_at) \
             VALUES ($1, $2, $3, $4, now() + INTERVAL '7 days', now() + INTERVAL '30 days')",
        )
        .bind(session_id.to_bytes().to_vec())
        .bind(user_id.to_bytes().to_vec())
        .bind(tokens.session_hash().to_vec())
        .bind(tokens.csrf_hash().to_vec())
        .execute(&pool)
        .await
        .unwrap();

        let response = app(SecurityState {
            pool,
            origin: "https://m1z.jp".to_owned(),
        })
        .oneshot(
            Request::post("/api/v1/posts")
                .header("content-type", "application/json")
                .header("origin", "https://m1z.jp")
                .header("x-csrf-token", &tokens.csrf)
                .header("idempotency-key", "http-post-test")
                .header(
                    "cookie",
                    format!(
                        "__Host-miz_session={}; __Host-miz_csrf={}",
                        tokens.session, tokens.csrf
                    ),
                )
                .body(Body::from(r#"{"content":"hello\nworld"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "application/json"
        );
    }
}
