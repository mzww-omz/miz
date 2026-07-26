//! Database and external service adapters.

use sqlx::PgPool;

pub async fn database(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPool::connect(url).await
}
