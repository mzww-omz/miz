#[path = "../infrastructure/mod.rs"]
mod infrastructure;

#[tokio::main]
async fn main() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = infrastructure::database(&url)
        .await
        .expect("database must be reachable");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("database migrations must succeed");
}
