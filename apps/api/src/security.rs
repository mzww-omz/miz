use crate::{
    api::Problem,
    authorization::{Principal, Role},
};
use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use miz_api::domain::{SessionId, UserId};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::net::IpAddr;
use subtle::ConstantTimeEq;
use url::Url;

const SESSION_COOKIE: &str = "__Host-miz_session";
const CSRF_COOKIE: &str = "__Host-miz_csrf";
const CSRF_HEADER: &str = "x-csrf-token";
const SESSION_RATE_LIMIT_PER_MINUTE: i64 = 120;

#[derive(Clone)]
pub struct SecurityState {
    pub pool: PgPool,
    pub origin: String,
    pub smtp_addr: String,
    pub cursor_signing_key: Vec<u8>,
}

pub struct SessionTokens {
    pub session: String,
    pub csrf: String,
}

pub struct OAuthChallenge {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
}

impl OAuthChallenge {
    pub fn generate() -> Result<Self, getrandom::Error> {
        Ok(Self {
            state: random_token()?,
            nonce: random_token()?,
            verifier: random_token()?,
        })
    }

    pub fn pkce_challenge(&self) -> String {
        pkce_challenge(&self.verifier)
    }

    pub fn state_hash(&self) -> [u8; 32] {
        hash(self.state.as_bytes())
    }

    pub fn nonce_hash(&self) -> [u8; 32] {
        hash(self.nonce.as_bytes())
    }
}

impl SessionTokens {
    pub fn generate() -> Result<Self, getrandom::Error> {
        Ok(Self {
            session: random_token()?,
            csrf: random_token()?,
        })
    }

    pub fn session_hash(&self) -> [u8; 32] {
        hash(self.session.as_bytes())
    }

    pub fn csrf_hash(&self) -> [u8; 32] {
        hash(self.csrf.as_bytes())
    }

    pub fn set_cookie_headers(&self) -> [String; 2] {
        [
            format!(
                "{SESSION_COOKIE}={}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=2592000",
                self.session
            ),
            format!(
                "{CSRF_COOKIE}={}; Path=/; Secure; SameSite=Lax; Max-Age=2592000",
                self.csrf
            ),
        ]
    }
}

pub async fn require_session(
    State(state): State<SecurityState>,
    mut request: Request,
    next: Next,
) -> Result<Response, Problem> {
    let session_token = cookie(request.headers(), SESSION_COOKIE).ok_or_else(|| {
        Problem::new(
            StatusCode::UNAUTHORIZED,
            "auth_required",
            "Sign in required",
        )
    })?;
    let token_hash = hash(session_token.as_bytes()).to_vec();
    let identity: (Vec<u8>, Vec<u8>, String, Vec<u8>) = sqlx::query_as(
        "UPDATE sessions AS session SET last_seen_at = now(), idle_expires_at = LEAST(now() + INTERVAL '7 days', absolute_expires_at) \
         FROM users WHERE session.user_id = users.id AND users.status = 'active' AND session.token_hash = $1 AND session.revoked_at IS NULL \
         AND session.idle_expires_at > now() AND session.absolute_expires_at > now() \
         RETURNING session.user_id, session.id, users.role, session.csrf_token_hash",
    )
    .bind(token_hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| Problem::new(StatusCode::UNAUTHORIZED, "auth_required", "Session is invalid or expired"))?;
    let user_id: [u8; 16] = identity
        .0
        .try_into()
        .map_err(|_| internal_error("invalid user ID"))?;
    let session_id: [u8; 16] = identity
        .1
        .try_into()
        .map_err(|_| internal_error("invalid session ID"))?;
    let role = match identity.2.as_str() {
        "user" => Role::User,
        "moderator" => Role::Moderator,
        "administrator" => Role::Administrator,
        _ => return Err(internal_error("invalid role")),
    };

    enforce_csrf(request.method(), request.headers(), &state.origin)?;
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        let csrf = cookie(request.headers(), CSRF_COOKIE).ok_or_else(csrf_problem)?;
        if !verify_secret(csrf, &identity.3) {
            return Err(csrf_problem());
        }
    }
    enforce_rate_limit(
        &state.pool,
        "session",
        session_token,
        SESSION_RATE_LIMIT_PER_MINUTE,
    )
    .await?;
    request.extensions_mut().insert(Principal {
        user_id: UserId::from_bytes(user_id),
        session_id: SessionId::from_bytes(session_id),
        role,
    });
    Ok(next.run(request).await)
}

fn enforce_csrf(
    method: &Method,
    headers: &HeaderMap,
    expected_origin: &str,
) -> Result<(), Problem> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    let cookie_token = cookie(headers, CSRF_COOKIE);
    let header_token = headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok());
    let fetch_site_is_safe = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| matches!(value, "same-origin" | "same-site" | "none"));
    let valid = origin == Some(expected_origin)
        && fetch_site_is_safe
        && cookie_token.zip(header_token).is_some_and(|(left, right)| {
            left.len() == right.len() && bool::from(left.as_bytes().ct_eq(right.as_bytes()))
        });
    if valid { Ok(()) } else { Err(csrf_problem()) }
}

pub(crate) async fn enforce_rate_limit(
    pool: &PgPool,
    bucket: &str,
    key: &str,
    limit: i64,
) -> Result<(), Problem> {
    let count: i64 = sqlx::query_scalar(
        "INSERT INTO rate_limit_windows (bucket, bucket_key, window_started_at, request_count) \
         VALUES ($1, $2, date_trunc('minute', now()), 1) \
         ON CONFLICT (bucket, bucket_key, window_started_at) DO UPDATE \
         SET request_count = rate_limit_windows.request_count + 1 RETURNING request_count",
    )
    .bind(bucket)
    .bind(URL_SAFE_NO_PAD.encode(hash(key.as_bytes())))
    .fetch_one(pool)
    .await
    .map_err(internal_error)?;
    if count <= limit {
        Ok(())
    } else {
        Err(Problem::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Rate limit exceeded",
        )
        .with_retry_after(60))
    }
}

pub async fn create_or_rotate_session(
    pool: &PgPool,
    user_id: UserId,
    device_name: &str,
    current_session_token: Option<&str>,
) -> Result<SessionTokens, Problem> {
    if device_name.chars().count() > 100 {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "deviceName must contain at most 100 characters",
        ));
    }
    let tokens = SessionTokens::generate().map_err(internal_error)?;
    let session_id = SessionId::new().map_err(internal_error)?;
    let mut transaction = pool.begin().await.map_err(internal_error)?;
    sqlx::query("SELECT id FROM users WHERE id = $1 AND status = 'active' FOR UPDATE")
        .bind(user_id.to_bytes().to_vec())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?;
    if let Some(current) = current_session_token {
        let revoked = sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE token_hash = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(hash(current.as_bytes()).to_vec())
        .bind(user_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
        if revoked.rows_affected() != 1 {
            return Err(Problem::new(
                StatusCode::UNAUTHORIZED,
                "auth_required",
                "Current session is invalid or expired",
            ));
        }
    }
    let active_sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sessions WHERE user_id = $1 AND revoked_at IS NULL AND idle_expires_at > now() AND absolute_expires_at > now()",
    )
    .bind(user_id.to_bytes().to_vec())
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if active_sessions >= 10 {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "session_limit_reached",
            "End an existing session before signing in on another device",
        ));
    }
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, csrf_token_hash, device_name, idle_expires_at, absolute_expires_at) \
         VALUES ($1, $2, $3, $4, $5, now() + INTERVAL '7 days', now() + INTERVAL '30 days')",
    )
    .bind(session_id.to_bytes().to_vec())
    .bind(user_id.to_bytes().to_vec())
    .bind(tokens.session_hash().to_vec())
    .bind(tokens.csrf_hash().to_vec())
    .bind(device_name)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(tokens)
}

pub fn validate_outbound_url(value: &str, allowed_hosts: &[&str]) -> Result<Url, &'static str> {
    let url = Url::parse(value).map_err(|_| "invalid URL")?;
    if url.scheme() != "https" || url.username() != "" || url.password().is_some() {
        return Err("outbound URL must use HTTPS without user info");
    }
    let host = url.host_str().ok_or("outbound URL must have a host")?;
    if host.parse::<IpAddr>().is_ok() || !allowed_hosts.contains(&host) {
        return Err("outbound host is not allowed");
    }
    Ok(url)
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(hash(verifier.as_bytes()))
}

pub fn verify_secret(value: &str, expected_hash: &[u8]) -> bool {
    let candidate = hash(value.as_bytes());
    expected_hash.len() == candidate.len() && bool::from(expected_hash.ct_eq(candidate.as_slice()))
}

pub(crate) fn random_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn hash(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
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

fn csrf_problem() -> Problem {
    Problem::new(
        StatusCode::FORBIDDEN,
        "csrf_failed",
        "CSRF validation failed",
    )
}

fn internal_error(_error: impl std::fmt::Display) -> Problem {
    eprintln!("security storage operation failed");
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
    fn tokens_are_opaque_hashed_and_cookie_hardened() {
        let tokens = SessionTokens::generate().unwrap();
        assert_eq!(tokens.session.len(), 43);
        assert_eq!(tokens.csrf.len(), 43);
        assert_ne!(tokens.session, tokens.csrf);
        assert_ne!(tokens.session_hash(), tokens.csrf_hash());
        let cookies = tokens.set_cookie_headers();
        assert!(cookies[0].contains("Secure; HttpOnly; SameSite=Lax"));
        assert!(!cookies[1].contains("HttpOnly"));
        assert!(!cookies[0].contains(&URL_SAFE_NO_PAD.encode(tokens.session_hash())));
    }

    #[test]
    fn csrf_requires_same_origin_and_matching_token() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://m1z.jp".parse().unwrap());
        headers.insert(
            header::COOKIE,
            format!("{CSRF_COOKIE}=secret").parse().unwrap(),
        );
        headers.insert(CSRF_HEADER, "secret".parse().unwrap());
        assert!(enforce_csrf(&Method::POST, &headers, "https://m1z.jp").is_ok());
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(enforce_csrf(&Method::POST, &headers, "https://m1z.jp").is_err());
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(enforce_csrf(&Method::POST, &headers, "https://m1z.jp").is_err());
        assert!(enforce_csrf(&Method::GET, &HeaderMap::new(), "https://m1z.jp").is_ok());
    }

    #[test]
    fn oauth_challenge_has_state_nonce_and_s256_pkce() {
        let challenge = OAuthChallenge::generate().unwrap();
        assert_eq!(challenge.state.len(), 43);
        assert_eq!(challenge.nonce.len(), 43);
        assert_eq!(challenge.verifier.len(), 43);
        assert_ne!(challenge.state_hash(), challenge.nonce_hash());
        assert!(verify_secret(&challenge.state, &challenge.state_hash()));
        assert!(!verify_secret("wrong-state", &challenge.state_hash()));
        assert_eq!(challenge.pkce_challenge().len(), 43);
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn outbound_urls_are_https_and_allowlisted() {
        assert!(
            validate_outbound_url(
                "https://accounts.google.com/o/oauth2/v2/auth",
                &["accounts.google.com"]
            )
            .is_ok()
        );
        assert!(
            validate_outbound_url("http://accounts.google.com", &["accounts.google.com"]).is_err()
        );
        assert!(validate_outbound_url("https://127.0.0.1", &["127.0.0.1"]).is_err());
        assert!(validate_outbound_url("https://evil.example", &["accounts.google.com"]).is_err());
    }
}
