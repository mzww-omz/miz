use axum::{
    Json,
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::sync::OnceLock;

const OPENAPI_YAML: &str = include_str!("../../../openapi/openapi.yaml");

pub async fn openapi() -> Json<serde_json::Value> {
    static DOCUMENT: OnceLock<serde_json::Value> = OnceLock::new();
    Json(
        DOCUMENT
            .get_or_init(|| serde_yaml::from_str(OPENAPI_YAML).expect("OpenAPI YAML must be valid"))
            .clone(),
    )
}

pub async fn not_found(uri: Uri) -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        format!("No route matches {}", uri.path()),
    )
}

pub fn parse_if_match(headers: &HeaderMap) -> Result<i64, Problem> {
    let value = headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            Problem::new(
                StatusCode::PRECONDITION_REQUIRED,
                "precondition_required",
                "If-Match is required",
            )
        })?;
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .and_then(|value| value.parse().ok())
        .filter(|version: &i64| *version > 0)
        .ok_or_else(|| {
            Problem::new(
                StatusCode::PRECONDITION_FAILED,
                "version_conflict",
                "If-Match is invalid",
            )
        })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    r#type: Box<str>,
    title: Box<str>,
    status: u16,
    detail: Box<str>,
    code: Box<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<Box<str>>,
    #[serde(skip)]
    retry_after: Option<u64>,
}

impl Problem {
    pub fn new(status: StatusCode, code: &str, detail: impl Into<String>) -> Self {
        Self {
            r#type: format!("https://m1z.jp/problems/{code}").into_boxed_str(),
            title: status.canonical_reason().unwrap_or("Error").into(),
            status: status.as_u16(),
            detail: detail.into().into_boxed_str(),
            code: code.into(),
            request_id: None,
            retry_after: None,
        }
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let retry_after = self.retry_after;
        let mut response = (status, Json(self)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            "application/problem+json"
                .parse()
                .expect("valid media type"),
        );
        if let Some(seconds) = retry_after {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                seconds.to_string().parse().expect("valid Retry-After"),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_openapi_document_has_required_contract() {
        let document: serde_json::Value = serde_yaml::from_str(OPENAPI_YAML).unwrap();
        assert_eq!(document["openapi"], "3.1.0");
        assert!(document["paths"]["/api/v1/posts"].is_object());
        assert_eq!(
            document["components"]["schemas"]["ObjectId"]["pattern"],
            "^[0-9A-Za-z]{22}$"
        );
        assert!(document["components"]["schemas"]["Problem"].is_object());
    }

    #[tokio::test]
    async fn rate_limit_problem_sets_retry_after() {
        let response = Problem::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "Slow down")
            .with_retry_after(60)
            .into_response();
        assert_eq!(response.headers()[header::RETRY_AFTER], "60");
    }

    #[tokio::test]
    async fn problem_uses_rfc_media_type_and_camel_case() {
        let response =
            Problem::new(StatusCode::UNAUTHORIZED, "auth_required", "Sign in").into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["code"], "auth_required");
        assert!(json.get("requestId").is_none());
    }
}
