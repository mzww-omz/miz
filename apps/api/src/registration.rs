use crate::{
    api::Problem,
    profile::{UserResponse, load_user},
    security::{self, SecurityState},
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
};
use miz_api::domain::{Handle, UserId};
use serde::Deserialize;

const MINIMUM_AGE: i32 = 13;
const MINIMUM_PASSWORD_BYTES: usize = 12;
const MAXIMUM_PASSWORD_BYTES: usize = 128;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterRequest {
    username: Handle,
    password: String,
    display_name: String,
    birth_date: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRequest {
    username: Handle,
    password: String,
}

pub async fn register(
    State(state): State<SecurityState>,
    headers: HeaderMap,
    Json(input): Json<RegisterRequest>,
) -> Result<(StatusCode, HeaderMap, Json<UserResponse>), Problem> {
    require_same_origin(&headers, &state.origin)?;
    validate_password(&input.password)?;
    let display_name = input.display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > 50 {
        return Err(validation("displayName must contain 1 to 50 characters"));
    }
    validate_birth_date(&state, &input.birth_date).await?;

    let username = input.username.to_string();
    security::enforce_rate_limit(&state.pool, "registration", &username, 5).await?;
    let password_hash = tokio::task::spawn_blocking(move || hash_password(input.password))
        .await
        .map_err(internal_error)??;

    let user_id = UserId::new().map_err(internal_error)?;
    let tokens = security::SessionTokens::generate().map_err(internal_error)?;
    let session_id = miz_api::domain::SessionId::new().map_err(internal_error)?;
    let mut tx = state.pool.begin().await.map_err(internal_error)?;
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id.to_bytes().to_vec())
        .bind(display_name)
        .execute(&mut *tx)
        .await
        .map_err(map_conflict)?;
    sqlx::query(
        "INSERT INTO handles (value, normalized, user_id, is_current) VALUES ($1, $1, $2, true)",
    )
    .bind(&username)
    .bind(user_id.to_bytes().to_vec())
    .execute(&mut *tx)
    .await
    .map_err(map_conflict)?;
    sqlx::query("INSERT INTO password_credentials (user_id, password_hash) VALUES ($1, $2)")
        .bind(user_id.to_bytes().to_vec())
        .bind(password_hash)
        .execute(&mut *tx)
        .await
        .map_err(internal_error)?;
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
    tx.commit().await.map_err(internal_error)?;

    Ok((
        StatusCode::CREATED,
        cookie_headers(&tokens)?,
        Json(load_user(&state.pool, user_id).await?),
    ))
}

pub async fn login(
    State(state): State<SecurityState>,
    headers: HeaderMap,
    Json(input): Json<LoginRequest>,
) -> Result<(HeaderMap, Json<UserResponse>), Problem> {
    require_same_origin(&headers, &state.origin)?;
    validate_password(&input.password)?;
    let username = input.username.to_string();
    security::enforce_rate_limit(&state.pool, "login", &username, 10).await?;
    let credential: Option<(Vec<u8>, String)> = sqlx::query_as(
        "SELECT u.id, c.password_hash FROM users u \
         JOIN handles h ON h.user_id = u.id AND h.is_current \
         JOIN password_credentials c ON c.user_id = u.id \
         WHERE h.normalized = $1 AND u.status = 'active'",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    let stored_hash = credential.as_ref().map(|row| row.1.clone());
    let verified =
        tokio::task::spawn_blocking(move || verify_password(input.password, stored_hash))
            .await
            .map_err(internal_error)??;
    if !verified {
        return Err(invalid_credentials());
    }
    let bytes: [u8; 16] = credential
        .ok_or_else(invalid_credentials)?
        .0
        .try_into()
        .map_err(|_| internal_error("invalid user ID"))?;
    let user_id = UserId::from_bytes(bytes);
    let tokens =
        security::create_or_rotate_session(&state.pool, user_id, "Local browser", None).await?;
    Ok((
        cookie_headers(&tokens)?,
        Json(load_user(&state.pool, user_id).await?),
    ))
}

async fn validate_birth_date(state: &SecurityState, birth_date: &str) -> Result<(), Problem> {
    let old_enough: bool = sqlx::query_scalar(
        "SELECT $1::date <= current_date - make_interval(years => $2) AND $1::date >= DATE '1900-01-01'",
    )
    .bind(birth_date)
    .bind(MINIMUM_AGE)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| validation("birthDate must be a valid date for a user aged 13 or older"))?;
    if old_enough {
        Ok(())
    } else {
        Err(validation("You must be at least 13 years old"))
    }
}

fn validate_password(password: &str) -> Result<(), Problem> {
    if (MINIMUM_PASSWORD_BYTES..=MAXIMUM_PASSWORD_BYTES).contains(&password.len()) {
        Ok(())
    } else {
        Err(validation("password must contain 12 to 128 bytes"))
    }
}

fn hash_password(password: String) -> Result<String, Problem> {
    let mut salt = [0_u8; 16];
    getrandom::fill(&mut salt).map_err(internal_error)?;
    let salt = SaltString::encode_b64(&salt).map_err(internal_error)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(internal_error)
}

fn verify_password(password: String, stored_hash: Option<String>) -> Result<bool, Problem> {
    if let Some(stored_hash) = stored_hash {
        let parsed = PasswordHash::new(&stored_hash).map_err(internal_error)?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    } else {
        let salt = SaltString::encode_b64(b"miz-dummy-salt!!").map_err(internal_error)?;
        let _ = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(internal_error)?;
        Ok(false)
    }
}

fn require_same_origin(headers: &HeaderMap, expected: &str) -> Result<(), Problem> {
    let valid = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        == Some(expected)
        && headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            .is_none_or(|value| matches!(value, "same-origin" | "same-site" | "none"));
    if valid {
        Ok(())
    } else {
        Err(Problem::new(
            StatusCode::FORBIDDEN,
            "csrf_failed",
            "Request origin is not allowed",
        ))
    }
}

fn cookie_headers(tokens: &security::SessionTokens) -> Result<HeaderMap, Problem> {
    let mut headers = HeaderMap::new();
    for cookie in tokens.set_cookie_headers() {
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).map_err(internal_error)?,
        );
    }
    Ok(headers)
}

fn validation(detail: &str) -> Problem {
    Problem::new(StatusCode::BAD_REQUEST, "problem_validation_failed", detail)
}

fn invalid_credentials() -> Problem {
    Problem::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "Username or password is incorrect",
    )
}

fn map_conflict(error: sqlx::Error) -> Problem {
    if matches!(&error, sqlx::Error::Database(database) if database.constraint().is_some()) {
        Problem::new(
            StatusCode::CONFLICT,
            "handle_conflict",
            "The requested username is already in use",
        )
    } else {
        internal_error(error)
    }
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(%error, "credential operation failed");
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
    fn passwords_are_salted_and_verified() {
        let first = hash_password("correct horse battery staple".to_owned()).unwrap();
        let second = hash_password("correct horse battery staple".to_owned()).unwrap();
        assert_ne!(first, second);
        assert!(verify_password("correct horse battery staple".to_owned(), Some(first)).unwrap());
        assert!(!verify_password("wrong password".to_owned(), Some(second)).unwrap());
        assert!(!verify_password("unknown password".to_owned(), None).unwrap());
    }

    #[tokio::test]
    async fn username_registration_and_login_issue_sessions() {
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
            cursor_signing_key: vec![7; 32],
        };
        let username: Handle = format!("u_{}", &UserId::new().unwrap().to_string()[..12])
            .parse()
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "http://localhost:8080".parse().unwrap());
        let (_, cookies, _) = register(
            State(state.clone()),
            headers.clone(),
            Json(RegisterRequest {
                username: username.clone(),
                password: "correct horse battery staple".to_owned(),
                display_name: "Local User".to_owned(),
                birth_date: "2000-01-01".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(cookies.get_all(header::SET_COOKIE).iter().count(), 2);
        let (cookies, _) = login(
            State(state),
            headers,
            Json(LoginRequest {
                username,
                password: "correct horse battery staple".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(cookies.get_all(header::SET_COOKIE).iter().count(), 2);
    }
}
