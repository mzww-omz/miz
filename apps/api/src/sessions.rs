use crate::{api::Problem, authorization::Principal, security::SecurityState};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use miz_api::domain::SessionId;
use serde::Serialize;

const EXPIRED_SESSION_COOKIE: &str =
    "__Host-miz_session=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0";
const EXPIRED_CSRF_COOKIE: &str = "__Host-miz_csrf=; Path=/; Secure; SameSite=Lax; Max-Age=0";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    id: SessionId,
    device_name: String,
    created_at: String,
    last_seen_at: String,
    idle_expires_at: String,
    absolute_expires_at: String,
    current: bool,
}

pub async fn list_sessions(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<SessionResponse>>, Problem> {
    let recently_authenticated: bool = sqlx::query_scalar(
        "SELECT authenticated_at > now() - INTERVAL '12 hours' FROM sessions WHERE id = $1 AND user_id = $2",
    )
    .bind(principal.session_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error)?;
    if !recently_authenticated {
        return Err(Problem::new(
            StatusCode::FORBIDDEN,
            "reauthentication_required",
            "Recent authentication is required to view sessions",
        ));
    }

    let rows: Vec<(Vec<u8>, String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, device_name, to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), to_char(last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), to_char(idle_expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), to_char(absolute_expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') FROM sessions WHERE user_id = $1 AND revoked_at IS NULL AND idle_expires_at > now() AND absolute_expires_at > now() ORDER BY last_seen_at DESC",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;
    rows.into_iter()
        .map(|row| {
            let id = row
                .0
                .as_slice()
                .try_into()
                .map(SessionId::from_bytes)
                .map_err(|_| internal_error("invalid session ID"))?;
            Ok(SessionResponse {
                id,
                device_name: row.1,
                created_at: row.2,
                last_seen_at: row.3,
                idle_expires_at: row.4,
                absolute_expires_at: row.5,
                current: id == principal.session_id,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

pub async fn revoke_session(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(session_id): Path<SessionId>,
) -> Result<Response, Problem> {
    let revoked: Option<Vec<u8>> = sqlx::query_scalar(
        "WITH revoked AS (UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL RETURNING id) INSERT INTO security_audit_log (actor_user_id, action, resource_type, resource_id) SELECT $2, 'session.revoked', 'session', id FROM revoked RETURNING resource_id",
    )
    .bind(session_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    if revoked.is_none() {
        return Err(Problem::new(
            StatusCode::NOT_FOUND,
            "resource_not_found",
            "Session not found",
        ));
    }
    Ok(no_content(session_id == principal.session_id))
}

pub async fn revoke_current_session(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, Problem> {
    revoke_owned_sessions(&state, principal, Some(principal.session_id)).await?;
    Ok(no_content(true))
}

pub async fn revoke_all_sessions(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
) -> Result<Response, Problem> {
    revoke_owned_sessions(&state, principal, None).await?;
    Ok(no_content(true))
}

async fn revoke_owned_sessions(
    state: &SecurityState,
    principal: Principal,
    session_id: Option<SessionId>,
) -> Result<(), Problem> {
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    match session_id {
        Some(session_id) => {
            sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL")
                .bind(session_id.to_bytes().to_vec())
                .bind(principal.user_id.to_bytes().to_vec())
                .execute(&mut *transaction)
                .await
                .map_err(internal_error)?;
        }
        None => {
            sqlx::query(
                "UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
            )
            .bind(principal.user_id.to_bytes().to_vec())
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
        }
    }
    sqlx::query("INSERT INTO security_audit_log (actor_user_id, action, resource_type, resource_id) VALUES ($1, $2, 'session', $3)")
        .bind(principal.user_id.to_bytes().to_vec())
        .bind(if session_id.is_some() { "session.revoked" } else { "sessions.revoked_all" })
        .bind(session_id.map(|id| id.to_bytes().to_vec()))
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)
}

fn no_content(clear_cookies: bool) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    if clear_cookies {
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_static(EXPIRED_SESSION_COOKIE),
        );
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_static(EXPIRED_CSRF_COOKIE),
        );
    }
    response
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

    #[test]
    fn logout_expires_both_cookies() {
        let response = no_content(true);
        let cookies: Vec<_> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .collect();
        assert_eq!(cookies.len(), 2);
        assert!(
            cookies
                .iter()
                .all(|cookie| cookie.to_str().unwrap().contains("Max-Age=0"))
        );
    }
}
