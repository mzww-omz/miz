use crate::{api::Problem, security, security::SecurityState};
use axum::{
    Json,
    extract::{Extension, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use miz_api::{
    domain::{OperatorId, OperatorRole, SessionId},
    operator_security,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

const OPERATOR_SESSION_COOKIE: &str = "__Host-miz_operator_session";
const OPERATOR_CSRF_COOKIE: &str = "__Host-miz_operator_csrf";
const CSRF_HEADER: &str = "x-csrf-token";

#[derive(Clone, Debug)]
pub struct OperatorPrincipal {
    pub operator_id: OperatorId,
    pub session_id: SessionId,
    pub roles: Vec<OperatorRole>,
    pub recent_mfa: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorLoginRequest {
    username: String,
    password: String,
    totp_code: Option<String>,
    recovery_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorResponse {
    id: OperatorId,
    username: String,
    roles: Vec<String>,
    recent_mfa: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorMfaRequest {
    totp_code: Option<String>,
    recovery_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorEnrollmentRequest {
    pub(crate) enrollment_token: String,
    pub(crate) totp_code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodesResponse {
    recovery_codes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorSessionResponse {
    id: SessionId,
    current: bool,
    last_seen_at: String,
    idle_expires_at: String,
    absolute_expires_at: String,
}

type OperatorLoginRow = (Vec<u8>, String, String, Vec<u8>, Vec<u8>, Option<i64>);

pub async fn login(
    State(state): State<SecurityState>,
    headers: HeaderMap,
    Json(input): Json<OperatorLoginRequest>,
) -> Result<(HeaderMap, Json<OperatorResponse>), Problem> {
    require_same_origin(&headers, &state.origin)?;
    if input.totp_code.is_some() == input.recovery_code.is_some() {
        return Err(validation("Provide exactly one MFA code"));
    }
    let username = normalize_username(&input.username)?;
    security::enforce_rate_limit(&state.pool, "operator-login", &username, 5).await?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let identity: Option<OperatorLoginRow> = sqlx::query_as(
        "SELECT a.id, a.username, c.password_hash, m.encrypted_totp_secret, m.encryption_nonce, m.last_used_step \
         FROM operator_accounts a \
         JOIN operator_credentials c ON c.operator_id = a.id \
         JOIN operator_mfa_factors m ON m.operator_id = a.id \
         WHERE a.normalized_username = $1 AND a.status = 'active' FOR UPDATE OF a, m",
    )
    .bind(&username)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let password_hash = identity
        .as_ref()
        .map(|row| row.2.clone())
        .unwrap_or_else(|| operator_security::dummy_password_hash().to_owned());
    let password = input.password;
    let password_valid = tokio::task::spawn_blocking(move || {
        operator_security::verify_password(&password, &password_hash)
    })
    .await
    .map_err(internal_error)?;
    let Some(identity) = identity.filter(|_| password_valid) else {
        return Err(invalid_credentials());
    };
    let operator_id = operator_id(&identity.0)?;

    if let Some(code) = input.totp_code {
        let secret = operator_security::decrypt_totp_secret(
            &state.operator_mfa_key,
            &identity.3,
            &identity.4,
        )
        .map_err(internal_error)?;
        let step = operator_security::verify_totp(&secret, &code, unix_seconds())
            .filter(|step| identity.5.is_none_or(|last| *step > last as u64))
            .ok_or_else(invalid_credentials)?;
        let updated = sqlx::query(
            "UPDATE operator_mfa_factors SET last_used_step = $2 WHERE operator_id = $1 \
             AND (last_used_step IS NULL OR last_used_step < $2)",
        )
        .bind(operator_id.to_bytes().to_vec())
        .bind(step as i64)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
        if updated.rows_affected() != 1 {
            return Err(invalid_credentials());
        }
    } else if let Some(code) = input.recovery_code {
        let consumed = sqlx::query(
            "UPDATE operator_recovery_codes SET used_at = now() \
             WHERE operator_id = $1 AND code_hash = $2 AND used_at IS NULL",
        )
        .bind(operator_id.to_bytes().to_vec())
        .bind(operator_security::recovery_code_hash(&code).to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
        if consumed.rows_affected() != 1 {
            return Err(invalid_credentials());
        }
    }

    let tokens = security::SessionTokens::generate().map_err(internal_error)?;
    let session_id = SessionId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO operator_sessions \
         (id, operator_id, token_hash, csrf_token_hash, mfa_verified_at, idle_expires_at, absolute_expires_at) \
         VALUES ($1, $2, $3, $4, now(), now() + INTERVAL '1 hour', now() + INTERVAL '12 hours')",
    )
    .bind(session_id.to_bytes().to_vec())
    .bind(operator_id.to_bytes().to_vec())
    .bind(tokens.session_hash().to_vec())
    .bind(tokens.csrf_hash().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let audit_request_id = miz_api::domain::RequestId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO audit_log_entries (actor_operator_id, event_type, target_type, target_id, request_id) \
         VALUES ($1, 'operatorSignIn', 'operator', $1, $2)",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(audit_request_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let roles = load_roles(&mut *transaction, operator_id).await?;
    transaction.commit().await.map_err(internal_error)?;

    Ok((
        operator_cookie_headers(&tokens)?,
        Json(OperatorResponse {
            id: operator_id,
            username: identity.1,
            roles: roles.iter().map(role_name).map(str::to_owned).collect(),
            recent_mfa: true,
        }),
    ))
}

pub async fn require_operator_session(
    State(state): State<SecurityState>,
    mut request: Request,
    next: Next,
) -> Result<Response, Problem> {
    let token =
        cookie(request.headers(), OPERATOR_SESSION_COOKIE).ok_or_else(operator_auth_required)?;
    let identity: (Vec<u8>, Vec<u8>, Vec<u8>, bool) = sqlx::query_as(
        "UPDATE operator_sessions AS session \
         SET last_seen_at = now(), idle_expires_at = LEAST(now() + INTERVAL '1 hour', absolute_expires_at) \
         FROM operator_accounts account \
         WHERE session.operator_id = account.id AND account.status = 'active' \
         AND session.token_hash = $1 AND session.revoked_at IS NULL \
         AND session.idle_expires_at > now() AND session.absolute_expires_at > now() \
         RETURNING session.operator_id, session.id, session.csrf_token_hash, session.mfa_verified_at > now() - INTERVAL '10 minutes'",
    )
    .bind(security::hash(token.as_bytes()).to_vec())
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(operator_auth_required)?;
    verify_operator_csrf(
        request.method(),
        request.headers(),
        &state.origin,
        &identity.2,
    )?;
    security::enforce_rate_limit(&state.pool, "operator-session", token, 120).await?;
    let operator_id = operator_id(&identity.0)?;
    let roles = load_roles(&state.pool, operator_id).await?;
    if roles.is_empty() {
        return Err(operator_permission_denied());
    }
    request.extensions_mut().insert(OperatorPrincipal {
        operator_id,
        session_id: session_id(&identity.1)?,
        roles,
        recent_mfa: identity.3,
    });
    Ok(next.run(request).await)
}

pub async fn current_operator(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
) -> Result<Json<OperatorResponse>, Problem> {
    let username: String = sqlx::query_scalar(
        "SELECT username FROM operator_accounts WHERE id = $1 AND status = 'active'",
    )
    .bind(principal.operator_id.to_bytes().to_vec())
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(operator_auth_required)?;
    Ok(Json(OperatorResponse {
        id: principal.operator_id,
        username,
        roles: principal
            .roles
            .iter()
            .map(role_name)
            .map(str::to_owned)
            .collect(),
        recent_mfa: principal.recent_mfa,
    }))
}

pub async fn logout(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
) -> Result<Response, Problem> {
    sqlx::query("UPDATE operator_sessions SET revoked_at = now() WHERE id = $1")
        .bind(principal.session_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    for value in [
        format!("{OPERATOR_SESSION_COOKIE}=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"),
        format!("{OPERATOR_CSRF_COOKIE}=; Path=/; Secure; SameSite=Strict; Max-Age=0"),
    ] {
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&value).map_err(internal_error)?,
        );
    }
    Ok(response)
}

pub async fn enroll_mfa(
    State(state): State<SecurityState>,
    headers: HeaderMap,
    Json(input): Json<OperatorEnrollmentRequest>,
) -> Result<StatusCode, Problem> {
    require_same_origin(&headers, &state.origin)?;
    let enrollment_hash = security::hash(input.enrollment_token.as_bytes());
    let rate_key = enrollment_hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    security::enforce_rate_limit(&state.pool, "operator-mfa-enrollment", &rate_key, 5).await?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let row: (Vec<u8>, Vec<u8>, Vec<u8>, Option<i64>) = sqlx::query_as(
        "SELECT challenge.operator_id, factor.encrypted_totp_secret, factor.encryption_nonce, factor.last_used_step \
         FROM operator_mfa_enrollment_challenges challenge \
         JOIN operator_mfa_factors factor ON factor.operator_id = challenge.operator_id \
         JOIN operator_accounts account ON account.id = challenge.operator_id AND account.status = 'pending' \
         WHERE challenge.token_hash = $1 AND challenge.consumed_at IS NULL AND challenge.expires_at > now() \
         FOR UPDATE OF challenge, factor, account",
    )
    .bind(enrollment_hash.to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(operator_mfa_required)?;
    let operator_id = operator_id(&row.0)?;
    verify_mfa_factor(
        &mut transaction,
        &state,
        operator_id,
        &row.1,
        &row.2,
        row.3,
        Some(input.totp_code),
        None,
    )
    .await?;
    sqlx::query(
        "UPDATE operator_mfa_enrollment_challenges SET consumed_at = now() WHERE operator_id = $1",
    )
    .bind(operator_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query("UPDATE operator_accounts SET status = 'active', updated_at = now() WHERE id = $1")
        .bind(operator_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    let audit_request_id = miz_api::domain::RequestId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO audit_log_entries (actor_operator_id, event_type, target_type, target_id, request_id) \
         VALUES ($1, 'operatorMfaEnrolled', 'operator', $1, $2)",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(audit_request_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn confirm_mfa(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Json(input): Json<OperatorMfaRequest>,
) -> Result<StatusCode, Problem> {
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let factor: (Vec<u8>, Vec<u8>, Option<i64>) = sqlx::query_as(
        "SELECT encrypted_totp_secret, encryption_nonce, last_used_step FROM operator_mfa_factors WHERE operator_id = $1 FOR UPDATE",
    )
    .bind(principal.operator_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(operator_mfa_required)?;
    verify_mfa_factor(
        &mut transaction,
        &state,
        principal.operator_id,
        &factor.0,
        &factor.1,
        factor.2,
        input.totp_code,
        input.recovery_code,
    )
    .await?;
    sqlx::query("UPDATE operator_sessions SET mfa_verified_at = now() WHERE id = $1")
        .bind(principal.session_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rotate_recovery_codes(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
) -> Result<Json<RecoveryCodesResponse>, Problem> {
    require_recent_mfa(&principal)?;
    let codes = operator_security::generate_recovery_codes().map_err(internal_error)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    sqlx::query("DELETE FROM operator_recovery_codes WHERE operator_id = $1")
        .bind(principal.operator_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    for code in &codes {
        sqlx::query("INSERT INTO operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)")
            .bind(principal.operator_id.to_bytes().to_vec())
            .bind(operator_security::recovery_code_hash(code).to_vec())
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
    }
    let audit_request_id = miz_api::domain::RequestId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO audit_log_entries (actor_operator_id, event_type, target_type, target_id, request_id) \
         VALUES ($1, 'operatorRecoveryCodesRotated', 'operator', $1, $2)",
    )
    .bind(principal.operator_id.to_bytes().to_vec())
    .bind(audit_request_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(RecoveryCodesResponse {
        recovery_codes: codes,
    }))
}

pub async fn list_sessions(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
) -> Result<Json<Vec<OperatorSessionResponse>>, Problem> {
    let rows: Vec<(Vec<u8>, String, String, String)> = sqlx::query_as(
        "SELECT id, \
         to_char(last_seen_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(idle_expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(absolute_expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM operator_sessions WHERE operator_id = $1 AND revoked_at IS NULL \
         AND idle_expires_at > now() AND absolute_expires_at > now() ORDER BY last_seen_at DESC",
    )
    .bind(principal.operator_id.to_bytes().to_vec())
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| {
                let id = session_id(&row.0)?;
                Ok(OperatorSessionResponse {
                    id,
                    current: id == principal.session_id,
                    last_seen_at: row.1,
                    idle_expires_at: row.2,
                    absolute_expires_at: row.3,
                })
            })
            .collect::<Result<_, Problem>>()?,
    ))
}

pub async fn revoke_session(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(session_id): Path<SessionId>,
) -> Result<StatusCode, Problem> {
    sqlx::query(
        "UPDATE operator_sessions SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1 AND operator_id = $2",
    )
    .bind(session_id.to_bytes().to_vec())
    .bind(principal.operator_id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[allow(clippy::too_many_arguments)]
async fn verify_mfa_factor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &SecurityState,
    operator_id: OperatorId,
    encrypted_secret: &[u8],
    nonce: &[u8],
    last_used_step: Option<i64>,
    totp_code: Option<String>,
    recovery_code: Option<String>,
) -> Result<(), Problem> {
    if totp_code.is_some() == recovery_code.is_some() {
        return Err(validation("Provide exactly one MFA code"));
    }
    if let Some(code) = totp_code {
        let secret = operator_security::decrypt_totp_secret(
            &state.operator_mfa_key,
            encrypted_secret,
            nonce,
        )
        .map_err(internal_error)?;
        let step = operator_security::verify_totp(&secret, &code, unix_seconds())
            .filter(|step| last_used_step.is_none_or(|last| *step > last as u64))
            .ok_or_else(invalid_credentials)?;
        let updated = sqlx::query(
            "UPDATE operator_mfa_factors SET last_used_step = $2 WHERE operator_id = $1 \
             AND (last_used_step IS NULL OR last_used_step < $2)",
        )
        .bind(operator_id.to_bytes().to_vec())
        .bind(step as i64)
        .execute(&mut **transaction)
        .await
        .map_err(internal_error)?;
        if updated.rows_affected() != 1 {
            return Err(invalid_credentials());
        }
    } else if let Some(code) = recovery_code {
        let consumed = sqlx::query(
            "UPDATE operator_recovery_codes SET used_at = now() \
             WHERE operator_id = $1 AND code_hash = $2 AND used_at IS NULL",
        )
        .bind(operator_id.to_bytes().to_vec())
        .bind(operator_security::recovery_code_hash(&code).to_vec())
        .execute(&mut **transaction)
        .await
        .map_err(internal_error)?;
        if consumed.rows_affected() != 1 {
            return Err(invalid_credentials());
        }
    }
    Ok(())
}

pub(crate) fn require_recent_mfa(principal: &OperatorPrincipal) -> Result<(), Problem> {
    if principal.recent_mfa {
        Ok(())
    } else {
        Err(Problem::new(
            StatusCode::FORBIDDEN,
            "operator_mfa_stale",
            "Recent MFA confirmation is required",
        ))
    }
}

async fn load_roles<'e, E>(
    executor: E,
    operator_id: OperatorId,
) -> Result<Vec<OperatorRole>, Problem>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT role FROM operator_role_assignments WHERE operator_id = $1 ORDER BY role",
    )
    .bind(operator_id.to_bytes().to_vec())
    .fetch_all(executor)
    .await
    .map_err(internal_error)?;
    roles.into_iter().map(|role| parse_role(&role)).collect()
}

fn verify_operator_csrf(
    method: &Method,
    headers: &HeaderMap,
    expected_origin: &str,
    expected_hash: &[u8],
) -> Result<(), Problem> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    require_same_origin(headers, expected_origin)?;
    let cookie = cookie(headers, OPERATOR_CSRF_COOKIE).ok_or_else(csrf_failed)?;
    let header = headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(csrf_failed)?;
    if cookie.len() == header.len()
        && bool::from(cookie.as_bytes().ct_eq(header.as_bytes()))
        && security::verify_secret(cookie, expected_hash)
    {
        Ok(())
    } else {
        Err(csrf_failed())
    }
}

fn require_same_origin(headers: &HeaderMap, expected_origin: &str) -> Result<(), Problem> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    let fetch_site_safe = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| matches!(value, "same-origin" | "same-site" | "none"));
    if origin == Some(expected_origin) && fetch_site_safe {
        Ok(())
    } else {
        Err(csrf_failed())
    }
}

fn operator_cookie_headers(tokens: &security::SessionTokens) -> Result<HeaderMap, Problem> {
    let mut headers = HeaderMap::new();
    for value in [
        format!(
            "{OPERATOR_SESSION_COOKIE}={}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=43200",
            tokens.session
        ),
        format!(
            "{OPERATOR_CSRF_COOKIE}={}; Path=/; Secure; SameSite=Strict; Max-Age=43200",
            tokens.csrf
        ),
    ] {
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_str(&value).map_err(internal_error)?,
        );
    }
    Ok(headers)
}

pub(crate) fn normalize_username(value: &str) -> Result<String, Problem> {
    let normalized = value.trim().to_ascii_lowercase();
    if (3..=64).contains(&normalized.len())
        && normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Ok(normalized)
    } else {
        Err(validation("Invalid operator username"))
    }
}

pub(crate) fn parse_role(value: &str) -> Result<OperatorRole, Problem> {
    match value {
        "support" => Ok(OperatorRole::Support),
        "moderator" => Ok(OperatorRole::Moderator),
        "seniorModerator" => Ok(OperatorRole::SeniorModerator),
        "administrator" => Ok(OperatorRole::Administrator),
        "auditor" => Ok(OperatorRole::Auditor),
        _ => Err(internal_error("invalid operator role")),
    }
}

pub(crate) fn role_name(role: &OperatorRole) -> &'static str {
    match role {
        OperatorRole::Support => "support",
        OperatorRole::Moderator => "moderator",
        OperatorRole::SeniorModerator => "seniorModerator",
        OperatorRole::Administrator => "administrator",
        OperatorRole::Auditor => "auditor",
    }
}

fn operator_id(bytes: &[u8]) -> Result<OperatorId, Problem> {
    bytes
        .try_into()
        .map(OperatorId::from_bytes)
        .map_err(|_| internal_error("invalid operator ID"))
}

fn session_id(bytes: &[u8]) -> Result<SessionId, Problem> {
    bytes
        .try_into()
        .map(SessionId::from_bytes)
        .map_err(|_| internal_error("invalid operator session ID"))
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_secs()
}

fn validation(detail: &str) -> Problem {
    Problem::new(StatusCode::BAD_REQUEST, "problem_validation_failed", detail)
}

fn invalid_credentials() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "Credentials are invalid",
    )
}

fn operator_mfa_required() -> Problem {
    Problem::new(
        StatusCode::FORBIDDEN,
        "operator_mfa_required",
        "MFA enrollment is required",
    )
}

fn operator_auth_required() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "operator_auth_required",
        "Operator sign in required",
    )
}

pub(crate) fn operator_permission_denied() -> Problem {
    Problem::new(
        StatusCode::FORBIDDEN,
        "operator_permission_denied",
        "Operator permission denied",
    )
}

fn csrf_failed() -> Problem {
    Problem::new(
        StatusCode::FORBIDDEN,
        "csrf_failed",
        "CSRF validation failed",
    )
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(error = %error, "operator operation failed");
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An internal error occurred",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use miz_api::operator_security::{
        encrypt_totp_secret, generate_totp_secret, hash_password, recovery_code_hash, totp_code,
    };

    #[tokio::test]
    async fn operator_login_requires_non_replayed_mfa_and_recovery_codes_are_single_use() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![4; 32],
            operator_mfa_key: [4; 32],
        };
        let id = OperatorId::new().unwrap();
        let username = id.to_string().to_ascii_lowercase();
        let password = "correct horse battery staple";
        let secret = generate_totp_secret().unwrap();
        let (encrypted, nonce) = encrypt_totp_secret(&state.operator_mfa_key, &secret).unwrap();
        let recovery_code = "single-use-recovery-code";
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO operator_accounts (id, username, normalized_username) VALUES ($1, $2, $2)",
        )
        .bind(id.to_bytes().to_vec())
        .bind(&username)
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO operator_credentials (operator_id, password_hash) VALUES ($1, $2)",
        )
        .bind(id.to_bytes().to_vec())
        .bind(hash_password(password.to_owned()).unwrap())
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO operator_mfa_factors (operator_id, encrypted_totp_secret, encryption_nonce) VALUES ($1, $2, $3)",
        )
        .bind(id.to_bytes().to_vec())
        .bind(encrypted)
        .bind(nonce.to_vec())
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO operator_role_assignments (operator_id, role) VALUES ($1, 'moderator')",
        )
        .bind(id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query("INSERT INTO operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)")
            .bind(id.to_bytes().to_vec())
            .bind(recovery_code_hash(recovery_code).to_vec())
            .execute(&mut *transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://m1z.jp".parse().unwrap());
        let code = totp_code(&secret, unix_seconds());
        let (cookie_headers, Json(operator)) = login(
            State(state.clone()),
            headers.clone(),
            Json(OperatorLoginRequest {
                username: username.clone(),
                password: password.to_owned(),
                totp_code: Some(code.clone()),
                recovery_code: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(operator.id, id);
        assert!(operator.recent_mfa);
        assert_eq!(cookie_headers.get_all(header::SET_COOKIE).iter().count(), 2);

        let replay = login(
            State(state.clone()),
            headers.clone(),
            Json(OperatorLoginRequest {
                username: username.clone(),
                password: password.to_owned(),
                totp_code: Some(code),
                recovery_code: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(replay.into_response().status(), StatusCode::UNAUTHORIZED);

        let _ = login(
            State(state.clone()),
            headers.clone(),
            Json(OperatorLoginRequest {
                username: username.clone(),
                password: password.to_owned(),
                totp_code: None,
                recovery_code: Some(recovery_code.to_owned()),
            }),
        )
        .await
        .unwrap();
        let reused_recovery = login(
            State(state),
            headers,
            Json(OperatorLoginRequest {
                username,
                password: password.to_owned(),
                totp_code: None,
                recovery_code: Some(recovery_code.to_owned()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(
            reused_recovery.into_response().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
