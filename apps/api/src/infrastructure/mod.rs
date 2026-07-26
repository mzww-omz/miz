//! Database and external service adapters.

use sqlx::{PgPool, migrate::Migrator};
use std::path::Path;

const MIGRATIONS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");

pub async fn database(url: &str) -> Result<PgPool, sqlx::Error> {
    PgPool::connect(url).await
}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    Migrator::new(Path::new(MIGRATIONS_PATH))
        .await?
        .run(pool)
        .await
}
