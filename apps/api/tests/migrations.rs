#[tokio::test]
async fn migrations_apply_twice_and_create_core_tables() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    miz_api::infrastructure::migrate(&pool).await.unwrap();
    miz_api::infrastructure::migrate(&pool).await.unwrap();

    for table in [
        "users",
        "posts",
        "post_edit_history",
        "idempotency_keys",
        "follow_relationships",
        "sessions",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exists, "missing table {table}");
    }
}
