use miz_api::infrastructure;

#[tokio::main]
async fn main() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = infrastructure::database(&url)
        .await
        .expect("database must be reachable");
    infrastructure::migrate(&pool)
        .await
        .expect("database migrations must succeed");
}
