use crate::{
    api::Problem,
    profile::{UserResponse, load_user},
    security::{self, SecurityState, SessionTokens},
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
};
use miz_api::domain::{Handle, RegistrationId, SessionId, UserId};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

const REGISTRATION_TTL_MINUTES: i32 = 30;
const MINIMUM_AGE: i32 = 13;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRegistrationRequest {
    provider: Provider,
    email: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Provider {
    MagicLink,
    Google,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationState {
    registration_id: RegistrationId,
    status: &'static str,
    expires_at: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyMagicLinkRequest {
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrationProfile {
    handle: Handle,
    display_name: String,
    birth_date: String,
}

#[derive(Serialize)]
pub struct RegistrationStatus {
    status: &'static str,
}

pub async fn start_registration(
    State(state): State<SecurityState>,
    Json(input): Json<StartRegistrationRequest>,
) -> Result<(StatusCode, Json<RegistrationState>), Problem> {
    if !matches!(input.provider, Provider::MagicLink) {
        return Err(validation("Only Magic Link is available in the local MVP"));
    }
    let email = normalize_email(&input.email).ok_or_else(|| validation("email is invalid"))?;
    security::enforce_rate_limit(&state.pool, "registration", &email, 5).await?;

    let registration_id = RegistrationId::new().map_err(internal_error)?;
    let challenge_id = RegistrationId::new().map_err(internal_error)?;
    let token = security::random_token().map_err(internal_error)?;
    let expires_at: String = sqlx::query_scalar(
        "WITH attempt AS (\
           INSERT INTO registration_attempts (id, email_normalized, expires_at) \
           VALUES ($1, $2, now() + make_interval(mins => $3)) RETURNING expires_at\
         ) \
         INSERT INTO auth_challenges (id, registration_id, kind, token_hash, expires_at) \
         SELECT $4, $1, 'magic_link', $5, expires_at FROM attempt \
         RETURNING to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .bind(registration_id.to_bytes().to_vec())
    .bind(&email)
    .bind(REGISTRATION_TTL_MINUTES)
    .bind(challenge_id.to_bytes().to_vec())
    .bind(security::hash(token.as_bytes()).to_vec())
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error)?;

    if let Err(error) = send_magic_link(&state, &email, registration_id, &token).await {
        let _ = sqlx::query("DELETE FROM registration_attempts WHERE id = $1")
            .bind(registration_id.to_bytes().to_vec())
            .execute(&state.pool)
            .await;
        return Err(internal_error(error));
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(RegistrationState {
            registration_id,
            status: "pending",
            expires_at,
        }),
    ))
}

pub async fn verify_magic_link(
    State(state): State<SecurityState>,
    Path(registration_id): Path<RegistrationId>,
    Json(input): Json<VerifyMagicLinkRequest>,
) -> Result<Json<RegistrationStatus>, Problem> {
    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    let row: Option<(Vec<u8>, bool, bool)> = sqlx::query_as(
        "SELECT token_hash, consumed_at IS NOT NULL, expires_at <= now() \
         FROM auth_challenges WHERE registration_id = $1 AND kind = 'magic_link' FOR UPDATE",
    )
    .bind(registration_id.to_bytes().to_vec())
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal_error)?;
    let Some((token_hash, consumed, expired)) = row else {
        return Err(invalid_magic_link());
    };
    if expired {
        return Err(expired_registration());
    }
    if consumed || !security::verify_secret(&input.token, &token_hash) {
        return Err(invalid_magic_link());
    }
    sqlx::query("UPDATE auth_challenges SET consumed_at = now() WHERE registration_id = $1")
        .bind(registration_id.to_bytes().to_vec())
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    sqlx::query("UPDATE registration_attempts SET status = 'verified' WHERE id = $1")
        .bind(registration_id.to_bytes().to_vec())
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    tx.commit().await.map_err(internal_error)?;
    Ok(Json(RegistrationStatus { status: "verified" }))
}

pub async fn save_profile(
    State(state): State<SecurityState>,
    Path(registration_id): Path<RegistrationId>,
    Json(input): Json<RegistrationProfile>,
) -> Result<Json<RegistrationStatus>, Problem> {
    let display_name = input.display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > 50 {
        return Err(validation("displayName must contain 1 to 50 characters"));
    }
    let old_enough: bool = sqlx::query_scalar(
        "SELECT $1::date <= current_date - make_interval(years => $2) AND $1::date >= DATE '1900-01-01'",
    )
    .bind(&input.birth_date)
    .bind(MINIMUM_AGE)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| validation("birthDate must be a valid date for a user aged 13 or older"))?;
    if !old_enough {
        return Err(validation("You must be at least 13 years old"));
    }

    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    ensure_registration(&mut tx, registration_id, &["verified", "profiled"]).await?;
    let handle = input.handle.to_string();
    let reserved: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM handles WHERE normalized = $1)")
            .bind(&handle)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal_error)?;
    if reserved {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "handle_conflict",
            "The requested handle is already in use",
        ));
    }
    sqlx::query(
        "INSERT INTO registration_profiles (registration_id, handle, display_name, birth_date) \
         VALUES ($1, $2, $3, $4::date) \
         ON CONFLICT (registration_id) DO UPDATE SET handle = EXCLUDED.handle, display_name = EXCLUDED.display_name, birth_date = EXCLUDED.birth_date",
    )
    .bind(registration_id.to_bytes().to_vec())
    .bind(handle)
    .bind(display_name)
    .bind(&input.birth_date)
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;
    sqlx::query("UPDATE registration_attempts SET status = 'profiled' WHERE id = $1")
        .bind(registration_id.to_bytes().to_vec())
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
    tx.commit().await.map_err(internal_error)?;
    Ok(Json(RegistrationStatus { status: "profiled" }))
}

pub async fn complete_registration(
    State(state): State<SecurityState>,
    Path(registration_id): Path<RegistrationId>,
) -> Result<(StatusCode, HeaderMap, Json<UserResponse>), Problem> {
    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    ensure_registration(&mut tx, registration_id, &["profiled"]).await?;
    let profile: (String, String, String) = sqlx::query_as(
        "SELECT p.handle, p.display_name, r.email_normalized \
         FROM registration_profiles p JOIN registration_attempts r ON r.id = p.registration_id \
         WHERE p.registration_id = $1",
    )
    .bind(registration_id.to_bytes().to_vec())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal_error)?;

    let user_id = UserId::new().map_err(internal_error)?;
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id.to_bytes().to_vec())
        .bind(&profile.1)
        .execute(&mut *tx)
        .await
        .map_err(map_account_conflict)?;
    sqlx::query(
        "INSERT INTO handles (value, normalized, user_id, is_current) VALUES ($1, $1, $2, true)",
    )
    .bind(&profile.0)
    .bind(user_id.to_bytes().to_vec())
    .execute(&mut *tx)
    .await
    .map_err(map_account_conflict)?;
    sqlx::query(
        "INSERT INTO auth_identities (user_id, provider, provider_subject, provider_email) VALUES ($1, 'magic_link', $2, $2)",
    )
    .bind(user_id.to_bytes().to_vec())
    .bind(&profile.2)
    .execute(&mut *tx)
    .await
    .map_err(map_account_conflict)?;

    let tokens = SessionTokens::generate().map_err(internal_error)?;
    let session_id = SessionId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, device_name, idle_expires_at, absolute_expires_at) \
         VALUES ($1, $2, $3, $4, 'Local browser', now() + INTERVAL '7 days', now() + INTERVAL '30 days')",
    )
    .bind(session_id.to_bytes().to_vec())
    .bind(user_id.to_bytes().to_vec())
    .bind(tokens.session_hash().to_vec())
    .bind(tokens.csrf_hash().to_vec())
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;
    sqlx::query(
        "UPDATE registration_attempts SET status = 'completed', completed_at = now() WHERE id = $1",
    )
    .bind(registration_id.to_bytes().to_vec())
    .execute(&mut *tx)
    .await
    .map_err(internal_error)?;
    tx.commit().await.map_err(internal_error)?;

    let mut headers = HeaderMap::new();
    for cookie in tokens.set_cookie_headers() {
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).map_err(internal_error)?,
        );
    }
    Ok((
        StatusCode::CREATED,
        headers,
        Json(load_user(&state.pool, user_id).await?),
    ))
}

async fn ensure_registration(
    tx: &mut Transaction<'_, Postgres>,
    id: RegistrationId,
    allowed: &[&str],
) -> Result<(), Problem> {
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT status, expires_at <= now() FROM registration_attempts WHERE id = $1 FOR UPDATE",
    )
    .bind(id.to_bytes().to_vec())
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal_error)?;
    match row {
        Some((_, true)) => Err(expired_registration()),
        Some((status, false)) if allowed.contains(&status.as_str()) => Ok(()),
        _ => Err(Problem::new(
            StatusCode::CONFLICT,
            "invalid_state_transition",
            "Registration is not ready for this step",
        )),
    }
}

fn normalize_email(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let (local, domain) = value.split_once('@')?;
    (value.len() <= 254
        && !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace()))
    .then_some(value)
}

async fn send_magic_link(
    state: &SecurityState,
    email: &str,
    registration_id: RegistrationId,
    token: &str,
) -> Result<(), std::io::Error> {
    let mut stream = TcpStream::connect(&state.smtp_addr).await?;
    expect_reply(&mut stream, 2).await?;
    smtp_command(&mut stream, "EHLO miz.local\r\n", 2).await?;
    smtp_command(&mut stream, "MAIL FROM:<noreply@miz.local>\r\n", 2).await?;
    smtp_command(&mut stream, &format!("RCPT TO:<{email}>\r\n"), 2).await?;
    smtp_command(&mut stream, "DATA\r\n", 3).await?;
    let link = format!(
        "{}/?registrationId={registration_id}&token={token}",
        state.origin.trim_end_matches('/')
    );
    let message = format!(
        "From: MIZ <noreply@miz.local>\r\nTo: <{email}>\r\nSubject: Your MIZ registration link\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nOpen this link to continue your MIZ registration:\r\n\r\n{link}\r\n\r\nThis link expires in 30 minutes.\r\n.\r\n"
    );
    smtp_command(&mut stream, &message, 2).await?;
    let _ = smtp_command(&mut stream, "QUIT\r\n", 2).await;
    Ok(())
}

async fn smtp_command(
    stream: &mut TcpStream,
    command: &str,
    expected: u8,
) -> Result<(), std::io::Error> {
    stream.write_all(command.as_bytes()).await?;
    expect_reply(stream, expected).await
}

async fn expect_reply(stream: &mut TcpStream, expected: u8) -> Result<(), std::io::Error> {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0
            || line.as_bytes().first().copied() != Some(b'0' + expected)
        {
            return Err(std::io::Error::other("SMTP server rejected the message"));
        }
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok(());
        }
    }
}

fn validation(detail: &str) -> Problem {
    Problem::new(StatusCode::BAD_REQUEST, "problem_validation_failed", detail)
}

fn invalid_magic_link() -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "magic_link_invalid_or_used",
        "Magic Link is invalid or has already been used",
    )
}

fn expired_registration() -> Problem {
    Problem::new(
        StatusCode::GONE,
        "registration_expired",
        "Registration has expired",
    )
}

fn map_account_conflict(error: sqlx::Error) -> Problem {
    if matches!(&error, sqlx::Error::Database(database) if database.constraint().is_some()) {
        Problem::new(
            StatusCode::CONFLICT,
            "handle_conflict",
            "The email or handle is already registered",
        )
    } else {
        internal_error(error)
    }
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(%error, "registration operation failed");
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
    fn email_validation_rejects_smtp_injection() {
        assert_eq!(
            normalize_email(" User@Example.COM ").as_deref(),
            Some("user@example.com")
        );
        assert!(normalize_email("user@example.com\r\nRCPT TO:<other@example.com>").is_none());
        assert!(normalize_email("not-an-email").is_none());
    }

    #[tokio::test]
    async fn verified_registration_creates_an_authenticated_account_once() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = miz_api::infrastructure::database(&database_url)
            .await
            .unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool,
            origin: "http://localhost:8080".to_owned(),
            smtp_addr: "127.0.0.1:1".to_owned(),
            cursor_signing_key: vec![7; 32],
        };
        let registration_id = RegistrationId::new().unwrap();
        let challenge_id = RegistrationId::new().unwrap();
        let token = security::random_token().unwrap();
        let email = format!("{}@example.com", registration_id);
        sqlx::query(
            "WITH attempt AS (\
               INSERT INTO registration_attempts (id, email_normalized, expires_at) VALUES ($1, $2, now() + INTERVAL '30 minutes') RETURNING expires_at\
             ) INSERT INTO auth_challenges (id, registration_id, kind, token_hash, expires_at) \
             SELECT $3, $1, 'magic_link', $4, expires_at FROM attempt",
        )
        .bind(registration_id.to_bytes().to_vec())
        .bind(email)
        .bind(challenge_id.to_bytes().to_vec())
        .bind(security::hash(token.as_bytes()).to_vec())
        .execute(&state.pool)
        .await
        .unwrap();

        let _ = verify_magic_link(
            State(state.clone()),
            Path(registration_id),
            Json(VerifyMagicLinkRequest {
                token: token.clone(),
            }),
        )
        .await
        .unwrap();
        assert!(
            verify_magic_link(
                State(state.clone()),
                Path(registration_id),
                Json(VerifyMagicLinkRequest { token }),
            )
            .await
            .is_err()
        );
        let handle: Handle = format!("u_{}", &registration_id.to_string()[..12])
            .parse()
            .unwrap();
        let _ = save_profile(
            State(state.clone()),
            Path(registration_id),
            Json(RegistrationProfile {
                handle,
                display_name: "Local User".to_owned(),
                birth_date: "2000-01-01".to_owned(),
            }),
        )
        .await
        .unwrap();
        let (_, headers, user) = complete_registration(State(state), Path(registration_id))
            .await
            .unwrap();
        assert_eq!(user.0.display_name, "Local User");
        assert_eq!(headers.get_all(header::SET_COOKIE).iter().count(), 2);
    }
}
