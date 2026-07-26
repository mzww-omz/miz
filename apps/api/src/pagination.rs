use axum::http::StatusCode;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use miz_api::domain::PostId;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::Problem;

type HmacSha256 = Hmac<Sha256>;
const CURSOR_TTL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    pub limit: Option<u16>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    scope: String,
    pub created_at: String,
    pub id: PostId,
    expires_at: u64,
}

pub fn page_limit(value: Option<u16>) -> Result<i64, Problem> {
    match value.unwrap_or(30) {
        1..=100 => Ok(i64::from(value.unwrap_or(30))),
        _ => Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "limit must contain a value from 1 to 100",
        )),
    }
}

pub fn encode(key: &[u8], scope: &str, created_at: String, id: PostId) -> Result<String, Problem> {
    let cursor = Cursor {
        scope: scope.to_owned(),
        created_at,
        id,
        expires_at: now()? + CURSOR_TTL_SECONDS,
    };
    let payload = serde_json::to_vec(&cursor).map_err(internal_error)?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(internal_error)?;
    mac.update(&payload);
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

pub fn decode(key: &[u8], scope: &str, value: &str) -> Result<Cursor, Problem> {
    if value.len() > 2048 {
        return Err(invalid_cursor());
    }
    let (payload, signature) = value.split_once('.').ok_or_else(invalid_cursor)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| invalid_cursor())?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| invalid_cursor())?;
    let mut mac = HmacSha256::new_from_slice(key).map_err(internal_error)?;
    mac.update(&payload);
    mac.verify_slice(&signature).map_err(|_| invalid_cursor())?;
    let cursor: Cursor = serde_json::from_slice(&payload).map_err(|_| invalid_cursor())?;
    if cursor.scope != scope {
        return Err(invalid_cursor());
    }
    if cursor.expires_at <= now()? {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "cursor_expired",
            "Cursor has expired",
        ));
    }
    Ok(cursor)
}

fn now() -> Result<u64, Problem> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(internal_error)
}

fn invalid_cursor() -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "invalid_cursor",
        "Cursor is invalid",
    )
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
    fn signed_cursor_rejects_tampering_and_wrong_scope() {
        let key = [7; 32];
        let id = PostId::from_bytes([3; 16]);
        let encoded = encode(&key, "timeline", "2026-07-26T00:00:00Z".to_owned(), id).unwrap();
        let cursor = decode(&key, "timeline", &encoded).unwrap();
        assert_eq!(cursor.id, id);
        assert!(decode(&key, "replies", &encoded).is_err());
        let mut tampered = encoded.into_bytes();
        tampered[0] = if tampered[0] == b'A' { b'B' } else { b'A' };
        assert!(decode(&key, "timeline", std::str::from_utf8(&tampered).unwrap()).is_err());
    }
}
