use crate::{
    api::{Problem, parse_if_match},
    authorization::Principal,
    security::SecurityState,
};
use axum::{
    Json,
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
};
use miz_api::domain::{Handle, UserId};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateUserRequest {
    pub handle: Option<Handle>,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub privacy: Option<Privacy>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserResponse {
    pub id: UserId,
    pub handle: Handle,
    pub display_name: String,
    pub bio: String,
    pub privacy: Privacy,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Privacy {
    Public,
    Private,
}

impl Privacy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

pub async fn get_current_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<UserResponse>, Problem> {
    Ok(Json(load_user(&state.pool, principal.user_id).await?))
}

pub async fn update_current_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, Problem> {
    let expected_version = parse_if_match(&headers)?;
    if input.display_name.as_deref().is_some_and(|value| {
        let trimmed = value.trim();
        trimmed.is_empty() || trimmed.chars().count() > 50
    }) {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "displayName must contain 1 to 50 characters",
        ));
    }
    if input
        .bio
        .as_deref()
        .is_some_and(|value| value.chars().count() > 160)
    {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "bio must contain at most 160 characters",
        ));
    }

    if input.handle.is_none()
        && input.display_name.is_none()
        && input.bio.is_none()
        && input.privacy.is_none()
    {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "At least one profile field is required",
        ));
    }

    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    let current: (i64,) = sqlx::query_as("SELECT version FROM users WHERE id = $1 FOR UPDATE")
        .bind(principal.user_id.to_bytes().to_vec())
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            Problem::new(
                StatusCode::NOT_FOUND,
                "resource_not_found",
                "User not found",
            )
        })?;
    if current.0 != expected_version {
        return Err(Problem::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            "The user was changed by another request",
        ));
    }

    if let Some(handle) = &input.handle {
        let old: (String, String, bool, Option<String>) = sqlx::query_as(
            "SELECT h.value, h.normalized, u.handle_changed_at IS NULL OR u.handle_changed_at <= now() - INTERVAL '30 days', CASE WHEN u.handle_changed_at IS NULL THEN NULL ELSE to_char((u.handle_changed_at + INTERVAL '30 days') AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') END FROM handles h JOIN users u ON u.id = h.user_id WHERE h.user_id = $1 AND h.is_current FOR UPDATE OF h, u",
        )
        .bind(principal.user_id.to_bytes().to_vec())
        .fetch_one(&mut *tx)
        .await
        .map_err(internal_error)?;
        if old.0 != handle.as_str() {
            if !old.2 {
                return Err(Problem::new(
                    StatusCode::CONFLICT,
                    "handle_change_too_soon",
                    format!(
                        "Handle can be changed after {}",
                        old.3
                            .as_deref()
                            .unwrap_or("the current restriction expires")
                    ),
                ));
            }
            if old.1 == handle.normalized() {
                sqlx::query("UPDATE handles SET value = $2 WHERE user_id = $1 AND is_current")
                    .bind(principal.user_id.to_bytes().to_vec())
                    .bind(handle.as_str())
                    .execute(&mut *tx)
                    .await
                    .map_err(internal_error)?;
            } else {
                sqlx::query("UPDATE handles SET is_current = false, retired_at = now() WHERE user_id = $1 AND is_current")
                    .bind(principal.user_id.to_bytes().to_vec())
                    .execute(&mut *tx)
                    .await
                    .map_err(internal_error)?;
                sqlx::query("INSERT INTO handles (user_id, value, normalized) VALUES ($1, $2, $3)")
                    .bind(principal.user_id.to_bytes().to_vec())
                    .bind(handle.as_str())
                    .bind(handle.normalized())
                    .execute(&mut *tx)
                    .await
                    .map_err(|error| map_conflict(error, "handle_conflict"))?;
            }
            sqlx::query("UPDATE users SET handle_changed_at = now() WHERE id = $1")
                .bind(principal.user_id.to_bytes().to_vec())
                .execute(&mut *tx)
                .await
                .map_err(internal_error)?;
        }
    }

    sqlx::query(
        "UPDATE users SET display_name = COALESCE($2, display_name), bio = COALESCE($3, bio), privacy = COALESCE($4, privacy), version = version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(input.display_name.map(|value| value.trim().to_owned()))
    .bind(input.bio)
    .bind(input.privacy.map(Privacy::as_str))
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;
    tx.commit().await.map_err(internal_error)?;
    Ok(Json(load_user(&state.pool, principal.user_id).await?))
}

pub(crate) async fn load_user(pool: &PgPool, user_id: UserId) -> Result<UserResponse, Problem> {
    let row: (Vec<u8>, String, String, String, String, i64, String, String) = sqlx::query_as(
        "SELECT u.id, h.value, u.display_name, u.bio, u.privacy, u.version, to_char(u.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), to_char(u.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') FROM users u JOIN handles h ON h.user_id = u.id AND h.is_current WHERE u.id = $1",
    )
    .bind(user_id.to_bytes().to_vec())
    .fetch_optional(pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| Problem::new(StatusCode::NOT_FOUND, "resource_not_found", "User not found"))?;
    Ok(UserResponse {
        id: row
            .0
            .as_slice()
            .try_into()
            .map(UserId::from_bytes)
            .map_err(|_| internal_error("invalid user ID"))?,
        handle: row
            .1
            .parse()
            .map_err(|_| internal_error("invalid handle"))?,
        display_name: row.2,
        bio: row.3,
        privacy: match row.4.as_str() {
            "public" => Privacy::Public,
            "private" => Privacy::Private,
            _ => return Err(internal_error("invalid privacy")),
        },
        version: row.5,
        created_at: row.6,
        updated_at: row.7,
    })
}

fn map_conflict(error: sqlx::Error, code: &str) -> Problem {
    if matches!(&error, sqlx::Error::Database(database) if database.constraint().is_some()) {
        Problem::new(
            StatusCode::CONFLICT,
            code,
            "The requested value is already in use",
        )
    } else {
        internal_error(error)
    }
}

fn internal_error(_error: impl std::fmt::Display) -> Problem {
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Request could not be completed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;

    #[test]
    fn parses_strong_etag_versions() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, "\"3\"".parse().unwrap());
        assert_eq!(parse_if_match(&headers).unwrap(), 3);
    }

    #[test]
    fn rejects_missing_or_weak_etags() {
        assert!(parse_if_match(&HeaderMap::new()).is_err());
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, "3".parse().unwrap());
        assert!(parse_if_match(&headers).is_err());
    }
}
