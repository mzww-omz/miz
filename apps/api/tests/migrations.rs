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
        "user_blocks",
        "user_mutes",
        "content_reports",
        "content_report_evidence",
        "operator_accounts",
        "operator_credentials",
        "operator_mfa_factors",
        "operator_recovery_codes",
        "operator_sessions",
        "operator_role_assignments",
        "moderation_actions",
        "audit_log_entries",
        "account_deletion_requests",
        "maintenance_jobs",
        "operator_mfa_enrollment_challenges",
        "user_restrictions",
        "moderation_appeals",
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

#[tokio::test]
async fn report_evidence_retention_is_retry_safe() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    miz_api::infrastructure::migrate(&pool).await.unwrap();
    let reporter = miz_api::domain::UserId::new().unwrap();
    let author = miz_api::domain::UserId::new().unwrap();
    for id in [reporter, author] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Retention user')")
            .bind(id.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
    }
    let expired_post = miz_api::domain::PostId::new().unwrap();
    let retained_post = miz_api::domain::PostId::new().unwrap();
    for id in [expired_post, retained_post] {
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'evidence', 'public')",
        )
        .bind(id.to_bytes().to_vec())
        .bind(author.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
    }
    for (post_id, retain_until) in [
        (expired_post, "now() - INTERVAL '1 second'"),
        (retained_post, "now() + INTERVAL '1 day'"),
    ] {
        let report_id = miz_api::domain::ReportId::new().unwrap();
        sqlx::query(
            "INSERT INTO content_reports (id, reporter_id, target_post_id, reason, state) VALUES ($1, $2, $3, 'spam', 'actioned')",
        )
        .bind(report_id.to_bytes().to_vec())
        .bind(reporter.to_bytes().to_vec())
        .bind(post_id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let sql = format!(
            "INSERT INTO content_report_evidence (report_id, target_kind, target_id, author_id, content, target_version, target_created_at, retain_until) \
             VALUES ($1, 'post', $2, $3, 'evidence', 1, now(), {retain_until})"
        );
        sqlx::query(&sql)
            .bind(report_id.to_bytes().to_vec())
            .bind(post_id.to_bytes().to_vec())
            .bind(author.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
    }

    assert_eq!(
        miz_api::infrastructure::purge_expired_report_evidence(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        miz_api::infrastructure::purge_expired_report_evidence(&pool)
            .await
            .unwrap(),
        0
    );
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM content_report_evidence WHERE retain_until > now()",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(retained >= 1);
}

#[tokio::test]
async fn audit_log_is_append_only() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    miz_api::infrastructure::migrate(&pool).await.unwrap();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO audit_log_entries (event_type) VALUES ('testEvent') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        sqlx::query("UPDATE audit_log_entries SET event_type = 'changed' WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM audit_log_entries WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );
    let expired: i64 = sqlx::query_scalar(
        "INSERT INTO audit_log_entries (event_type, retain_until) VALUES ('expiredTestEvent', now() - INTERVAL '1 second') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        miz_api::infrastructure::purge_expired_audit_logs(&pool)
            .await
            .unwrap()
            >= 1
    );
    let remains: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM audit_log_entries WHERE id = $1)")
            .bind(expired)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!remains);
}
