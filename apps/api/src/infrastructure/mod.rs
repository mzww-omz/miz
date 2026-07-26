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

pub async fn purge_expired_report_evidence(pool: &PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM content_report_evidence WHERE retain_until <= now()")
        .execute(pool)
        .await
        .map(|result| result.rows_affected())
}

pub async fn purge_expired_audit_logs(pool: &PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM audit_log_entries WHERE retain_until <= now()")
        .execute(pool)
        .await
        .map(|result| result.rows_affected())
}

pub async fn expire_temporary_restrictions(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let expired_users: Vec<Vec<u8>> = sqlx::query_scalar(
        "UPDATE user_restrictions SET revoked_at = now() \
         WHERE revoked_at IS NULL AND expires_at <= now() RETURNING user_id",
    )
    .fetch_all(&mut *transaction)
    .await?;
    for user_id in &expired_users {
        sqlx::query(
            "UPDATE users SET status = 'active', updated_at = now() WHERE id = $1 AND status = 'suspended' \
             AND NOT EXISTS (SELECT 1 FROM user_restrictions restriction WHERE restriction.user_id = users.id \
               AND restriction.kind IN ('temporarySuspension', 'permanentSuspension') \
               AND restriction.revoked_at IS NULL AND (restriction.expires_at IS NULL OR restriction.expires_at > now()))",
        )
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(expired_users.len() as u64)
}

pub async fn purge_expired_accounts(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let mut purged = 0;
    loop {
        let mut claim = pool.begin().await?;
        let claimed: Option<(Vec<u8>, Vec<u8>, i64)> = sqlx::query_as(
            "SELECT request.id, request.user_id, job.id \
             FROM account_deletion_requests request \
             JOIN maintenance_jobs job ON job.account_deletion_request_id = request.id \
             WHERE ((request.state = 'pending' AND request.restore_until <= now() \
                     AND job.state IN ('pending', 'failed') AND job.available_at <= now()) \
                OR (request.state = 'purging' AND \
                    ((job.state = 'failed' AND job.available_at <= now()) \
                     OR (job.state = 'claimed' AND job.claimed_at <= now() - INTERVAL '1 hour')))) \
             ORDER BY request.restore_until, request.id FOR UPDATE OF request, job SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *claim)
        .await?;
        let Some((request_id, user_id, job_id)) = claimed else {
            claim.rollback().await?;
            break;
        };
        sqlx::query(
            "UPDATE account_deletion_requests SET state = 'purging', claimed_at = COALESCE(claimed_at, now()) WHERE id = $1",
        )
        .bind(&request_id)
        .execute(&mut *claim)
        .await?;
        sqlx::query(
            "UPDATE maintenance_jobs SET state = 'claimed', attempts = attempts + 1, claimed_at = now(), last_error_code = NULL, updated_at = now() WHERE id = $1",
        )
        .bind(job_id)
        .execute(&mut *claim)
        .await?;
        claim.commit().await?;

        if let Err(error) = purge_claimed_account(pool, &request_id, &user_id, job_id).await {
            sqlx::query(
                "UPDATE maintenance_jobs SET state = 'failed', available_at = now() + INTERVAL '5 minutes', last_error_code = 'account_purge_failed', updated_at = now() WHERE id = $1",
            )
            .bind(job_id)
            .execute(pool)
            .await?;
            return Err(error);
        }
        purged += 1;
    }
    Ok(purged)
}

async fn purge_claimed_account(
    pool: &PgPool,
    request_id: &[u8],
    user_id: &[u8],
    job_id: i64,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM password_credentials WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM auth_identities WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE security_audit_log SET actor_user_id = NULL WHERE actor_user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM post_mentions WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE content_reports SET reporter_id = NULL WHERE reporter_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM follow_relationships WHERE follower_id = $1 OR followee_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 OR blocked_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM user_mutes WHERE muter_id = $1 OR muted_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "DELETE FROM post_edit_history WHERE editor_id = $1 OR post_id IN (SELECT id FROM posts WHERE author_id = $1)",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE posts SET author_id = decode(repeat('00', 16), 'hex'), content = NULL, \
         state = CASE WHEN EXISTS (SELECT 1 FROM posts child WHERE child.reply_to_post_id = posts.id) THEN 'tombstone' ELSE 'deleted' END, \
         version = version + 1, deleted_at = COALESCE(deleted_at, now()), updated_at = now() WHERE author_id = $1",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE handles SET is_current = false, retired_at = COALESCE(retired_at, now()) WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE users SET display_name = 'Deleted account', bio = '', privacy = 'private', status = 'deleted', version = version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE account_deletion_requests SET state = 'purged', completed_at = now() WHERE id = $1",
    )
    .bind(request_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE maintenance_jobs SET state = 'completed', completed_at = now(), last_error_code = NULL, updated_at = now() WHERE id = $1",
    )
    .bind(job_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}
