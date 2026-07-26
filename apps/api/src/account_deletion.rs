use crate::{
    api::Problem,
    authorization::Principal,
    registration::require_same_origin,
    security::{self, SecurityState},
};
use axum::{
    Json,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
};
use miz_api::{
    domain::{AccountDeletionRequestId, Handle, UserId},
    operator_security,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestDeletion {
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestoreAccount {
    username: Handle,
    password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeletionResponse {
    id: AccountDeletionRequestId,
    status: String,
    requested_at: String,
    restore_until: String,
}

type DeletionRow = (Vec<u8>, String, String, String);

pub async fn request_deletion(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<RequestDeletion>,
) -> Result<(StatusCode, Json<AccountDeletionResponse>), Problem> {
    require_same_origin(&headers, &state.origin)?;
    let credential: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM password_credentials WHERE user_id = $1")
            .bind(principal.user_id.to_bytes().to_vec())
            .fetch_optional(&state.pool)
            .await
            .map_err(internal_error)?;
    verify_password(input.password, credential).await?;

    let request_id = AccountDeletionRequestId::new().map_err(internal_error)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND status = 'active' FOR UPDATE)",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if !active {
        return Err(deletion_not_pending());
    }
    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM account_deletion_requests WHERE user_id = $1 AND state IN ('pending', 'purging'))",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if existing {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "deletion_already_pending",
            "Account deletion is already pending",
        ));
    }
    let row: DeletionRow = sqlx::query_as(
        "INSERT INTO account_deletion_requests (id, user_id, restore_until) \
         VALUES ($1, $2, now() + INTERVAL '30 days') \
         RETURNING id, state, \
         to_char(requested_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(restore_until AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .bind(request_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO maintenance_jobs (kind, account_deletion_request_id, available_at) \
         VALUES ('purgeAccount', $1, now() + INTERVAL '30 days')",
    )
    .bind(request_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query("UPDATE users SET status = 'deleted', updated_at = now() WHERE id = $1")
        .bind(principal.user_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    sqlx::query(
        "UPDATE sessions SET revoked_at = COALESCE(revoked_at, now()) WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok((StatusCode::CREATED, Json(deletion_response(row)?)))
}

pub async fn cancel_deletion(
    State(state): State<SecurityState>,
    headers: HeaderMap,
    Json(input): Json<RestoreAccount>,
) -> Result<Json<AccountDeletionResponse>, Problem> {
    restore(state, headers, input, "cancelled").await.map(Json)
}

pub async fn restore_account(
    State(state): State<SecurityState>,
    headers: HeaderMap,
    Json(input): Json<RestoreAccount>,
) -> Result<Json<AccountDeletionResponse>, Problem> {
    restore(state, headers, input, "restored").await.map(Json)
}

async fn restore(
    state: SecurityState,
    headers: HeaderMap,
    input: RestoreAccount,
    next_state: &str,
) -> Result<AccountDeletionResponse, Problem> {
    require_same_origin(&headers, &state.origin)?;
    let username = input.username.to_string();
    security::enforce_rate_limit(&state.pool, "account-restoration", &username, 5).await?;
    let identity: Option<(Vec<u8>, String)> = sqlx::query_as(
        "SELECT u.id, c.password_hash FROM users u \
         JOIN handles h ON h.user_id = u.id AND h.is_current \
         JOIN password_credentials c ON c.user_id = u.id \
         WHERE h.normalized = $1 AND u.status = 'deleted'",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    let credential = identity.as_ref().map(|row| row.1.clone());
    verify_password(input.password, credential).await?;
    let user_id = identity
        .and_then(|row| row.0.try_into().ok())
        .map(UserId::from_bytes)
        .ok_or_else(invalid_credentials)?;

    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let request: Option<(Vec<u8>, String, bool)> = sqlx::query_as(
        "SELECT id, state, restore_until > now() FROM account_deletion_requests \
         WHERE user_id = $1 ORDER BY requested_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(user_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let Some((request_id, request_state, within_window)) = request else {
        return Err(deletion_not_pending());
    };
    if request_state == "purging" {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "purge_in_progress",
            "Account purge is in progress",
        ));
    }
    if request_state != "pending" {
        return Err(deletion_not_pending());
    }
    if !within_window {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "restoration_window_expired",
            "The account restoration window has expired",
        ));
    }
    let row: DeletionRow = sqlx::query_as(
        "UPDATE account_deletion_requests SET state = $2, completed_at = now() WHERE id = $1 \
         RETURNING id, state, \
         to_char(requested_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(restore_until AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .bind(&request_id)
    .bind(next_state)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query("UPDATE users SET status = 'active', updated_at = now() WHERE id = $1")
        .bind(user_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    sqlx::query(
        "UPDATE maintenance_jobs SET state = 'completed', completed_at = now(), updated_at = now() \
         WHERE account_deletion_request_id = $1 AND state IN ('pending', 'failed')",
    )
    .bind(&request_id)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    deletion_response(row)
}

async fn verify_password(password: String, stored_hash: Option<String>) -> Result<(), Problem> {
    if password.is_empty() || password.len() > operator_security::MAXIMUM_PASSWORD_BYTES {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "password must contain 1 to 128 UTF-8 bytes",
        ));
    }
    let stored_hash =
        stored_hash.unwrap_or_else(|| operator_security::dummy_password_hash().into());
    let valid = tokio::task::spawn_blocking(move || {
        operator_security::verify_password(&password, &stored_hash)
    })
    .await
    .map_err(internal_error)?;
    if valid {
        Ok(())
    } else {
        Err(invalid_credentials())
    }
}

fn deletion_response(row: DeletionRow) -> Result<AccountDeletionResponse, Problem> {
    let id = row
        .0
        .try_into()
        .map(AccountDeletionRequestId::from_bytes)
        .map_err(|_| internal_error("invalid deletion request ID"))?;
    Ok(AccountDeletionResponse {
        id,
        status: row.1,
        requested_at: row.2,
        restore_until: row.3,
    })
}

fn invalid_credentials() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "Credentials are invalid",
    )
}

fn deletion_not_pending() -> Problem {
    Problem::new(
        StatusCode::CONFLICT,
        "deletion_not_pending",
        "Account deletion is not pending",
    )
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(error = %error, "account deletion operation failed");
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An internal error occurred",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use miz_api::domain::SessionId;

    #[tokio::test]
    async fn deletion_restores_within_grace_and_purges_after_deadline() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![6; 32],
            operator_mfa_key: [6; 32],
        };
        let user_id = UserId::new().unwrap();
        let username = user_id.to_string().to_ascii_lowercase();
        let password = "correct horse battery staple";
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Delete me')")
            .bind(user_id.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO handles (value, normalized, user_id, is_current) VALUES ($1, $1, $2, true)",
        )
        .bind(&username)
        .bind(user_id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO password_credentials (user_id, password_hash) VALUES ($1, $2)")
            .bind(user_id.to_bytes().to_vec())
            .bind(operator_security::hash_password(password.to_owned()).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let session_id = SessionId::new().unwrap();
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, idle_expires_at, absolute_expires_at) \
             VALUES ($1, $2, $3, $4, now() + INTERVAL '1 day', now() + INTERVAL '2 days')",
        )
        .bind(session_id.to_bytes().to_vec())
        .bind(user_id.to_bytes().to_vec())
        .bind([user_id.to_bytes(), [1; 16]].concat())
        .bind([user_id.to_bytes(), [2; 16]].concat())
        .execute(&pool)
        .await
        .unwrap();
        let post_id = miz_api::domain::PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'personal post', 'public')",
        )
        .bind(post_id.to_bytes().to_vec())
        .bind(user_id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let principal = Principal {
            user_id,
            session_id,
            role: crate::authorization::Role::User,
        };
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://m1z.jp".parse().unwrap());

        let (_, Json(pending)) = request_deletion(
            State(state.clone()),
            Extension(principal),
            headers.clone(),
            Json(RequestDeletion {
                password: password.to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(pending.status, "pending");
        let user_status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1")
            .bind(user_id.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(user_status, "deleted");
        let revoked: bool =
            sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM sessions WHERE id = $1")
                .bind(session_id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(revoked);

        let Json(restored) = restore_account(
            State(state.clone()),
            headers.clone(),
            Json(RestoreAccount {
                username: username.parse().unwrap(),
                password: password.to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(restored.status, "restored");
        let user_status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1")
            .bind(user_id.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(user_status, "active");

        let (_, Json(second)) = request_deletion(
            State(state.clone()),
            Extension(principal),
            headers,
            Json(RequestDeletion {
                password: password.to_owned(),
            }),
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE account_deletion_requests SET requested_at = now() - INTERVAL '31 days', restore_until = now() - INTERVAL '1 day' WHERE id = $1",
        )
        .bind(second.id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE maintenance_jobs SET available_at = now() - INTERVAL '1 day' WHERE account_deletion_request_id = $1",
        )
        .bind(second.id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let (first_worker, second_worker) = tokio::join!(
            miz_api::infrastructure::purge_expired_accounts(&pool),
            miz_api::infrastructure::purge_expired_accounts(&pool),
        );
        assert_eq!(first_worker.unwrap() + second_worker.unwrap(), 1);
        assert_eq!(
            miz_api::infrastructure::purge_expired_accounts(&pool)
                .await
                .unwrap(),
            0
        );
        let purged: String =
            sqlx::query_scalar("SELECT state FROM account_deletion_requests WHERE id = $1")
                .bind(second.id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(purged, "purged");
        let credential_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM password_credentials WHERE user_id = $1")
                .bind(user_id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(credential_count, 0);
        let post: (Vec<u8>, Option<String>, String) =
            sqlx::query_as("SELECT author_id, content, state FROM posts WHERE id = $1")
                .bind(post_id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(post.0, vec![0; 16]);
        assert!(post.1.is_none());
        assert_eq!(post.2, "deleted");
    }
}
