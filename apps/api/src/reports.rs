use crate::{
    api::{Problem, parse_if_match},
    authorization::Principal,
    posts::load_post,
    relationships::lock_user_pair,
    security::SecurityState,
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
};
use miz_api::domain::{PostId, ReportId, UserId};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportReason {
    Spam,
    Harassment,
    HatefulContent,
    Violence,
    SexualContent,
    IllegalOrDangerousTrade,
    PersonalInformation,
    Copyright,
    Other,
}

impl ReportReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Spam => "spam",
            Self::Harassment => "harassment",
            Self::HatefulContent => "hatefulContent",
            Self::Violence => "violence",
            Self::SexualContent => "sexualContent",
            Self::IllegalOrDangerousTrade => "illegalOrDangerousTrade",
            Self::PersonalInformation => "personalInformation",
            Self::Copyright => "copyright",
            Self::Other => "other",
        }
    }

    fn parse(value: &str) -> Result<Self, Problem> {
        match value {
            "spam" => Ok(Self::Spam),
            "harassment" => Ok(Self::Harassment),
            "hatefulContent" => Ok(Self::HatefulContent),
            "violence" => Ok(Self::Violence),
            "sexualContent" => Ok(Self::SexualContent),
            "illegalOrDangerousTrade" => Ok(Self::IllegalOrDangerousTrade),
            "personalInformation" => Ok(Self::PersonalInformation),
            "copyright" => Ok(Self::Copyright),
            "other" => Ok(Self::Other),
            _ => Err(internal_error("invalid report reason")),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateReportRequest {
    reason: ReportReason,
    explanation: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateReportRequest {
    reason: Option<ReportReason>,
    explanation: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportResponse {
    id: ReportId,
    target_id: PostId,
    target_type: String,
    reason: ReportReason,
    explanation: Option<String>,
    status: String,
    version: i64,
    created_at: String,
    updated_at: String,
}

type ReportRow = (
    Vec<u8>,
    Vec<u8>,
    String,
    String,
    Option<String>,
    String,
    i64,
    String,
    String,
);

pub async fn create_report(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(post_id): Path<PostId>,
    Json(input): Json<CreateReportRequest>,
) -> Result<(StatusCode, Json<ReportResponse>), Problem> {
    let explanation = validate_explanation(input.reason, input.explanation)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let mut target = load_post(&mut transaction, post_id, principal).await?;
    lock_user_pair(&mut transaction, principal.user_id, target.author_id).await?;
    target = load_post(&mut transaction, post_id, principal).await?;
    if target.author_id == principal.user_id {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "cannot_report_own_content",
            "A user cannot report their own content",
        ));
    }
    let content = target.content.ok_or_else(report_target_not_visible)?;
    if target.state != "published" {
        return Err(report_target_not_visible());
    }

    if let Some(report) =
        load_unresolved_report(&mut transaction, principal.user_id, post_id).await?
    {
        transaction.commit().await.map_err(internal_error)?;
        return Ok((StatusCode::OK, Json(report)));
    }

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(encode($1::bytea, 'hex'), 0))")
        .bind(principal.user_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    if let Some(report) =
        load_unresolved_report(&mut transaction, principal.user_id, post_id).await?
    {
        transaction.commit().await.map_err(internal_error)?;
        return Ok((StatusCode::OK, Json(report)));
    }
    let recent: (i64, Option<i64>) = sqlx::query_as(
        "SELECT count(*), ceil(extract(epoch FROM min(created_at + INTERVAL '1 hour' - now())))::bigint \
         FROM content_reports WHERE reporter_id = $1 AND created_at > now() - INTERVAL '1 hour'",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if recent.0 >= 20 {
        return Err(Problem::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Report rate limit exceeded",
        )
        .with_retry_after(recent.1.unwrap_or(60).max(1) as u64));
    }

    let report_id = ReportId::new().map_err(internal_error)?;
    let inserted: Option<Vec<u8>> = sqlx::query_scalar(
        "INSERT INTO content_reports (id, reporter_id, target_post_id, reason, explanation) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (reporter_id, target_post_id) WHERE state IN ('received', 'inReview') DO NOTHING \
         RETURNING id",
    )
    .bind(report_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(post_id.to_bytes().to_vec())
    .bind(input.reason.as_str())
    .bind(explanation)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let status = if inserted.is_some() {
        sqlx::query(
            "INSERT INTO content_report_evidence \
             (report_id, target_kind, target_id, author_id, content, target_version, target_created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)",
        )
        .bind(report_id.to_bytes().to_vec())
        .bind(if target.reply_to_post_id.is_some() {
            "reply"
        } else {
            "post"
        })
        .bind(post_id.to_bytes().to_vec())
        .bind(target.author_id.to_bytes().to_vec())
        .bind(content)
        .bind(target.version)
        .bind(target.created_at)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let report = load_unresolved_report(&mut transaction, principal.user_id, post_id)
        .await?
        .ok_or_else(|| internal_error("report insert did not return a row"))?;
    transaction.commit().await.map_err(internal_error)?;
    Ok((status, Json(report)))
}

pub async fn get_report(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(report_id): Path<ReportId>,
) -> Result<Json<ReportResponse>, Problem> {
    load_report(&state.pool, principal.user_id, report_id)
        .await
        .map(Json)
}

pub async fn update_report(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(report_id): Path<ReportId>,
    headers: HeaderMap,
    Json(input): Json<UpdateReportRequest>,
) -> Result<Json<ReportResponse>, Problem> {
    let expected_version = parse_if_match(&headers)?;
    if input.reason.is_none() && input.explanation.is_none() {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "At least one report field is required",
        ));
    }
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: (String, Option<String>, String, i64) = sqlx::query_as(
        "SELECT reason, explanation, state, version FROM content_reports \
         WHERE id = $1 AND reporter_id = $2 FOR UPDATE",
    )
    .bind(report_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(report_not_found)?;
    if !matches!(current.2.as_str(), "received" | "inReview") {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "report_not_editable",
            "Resolved reports cannot be edited",
        ));
    }
    if current.3 != expected_version {
        return Err(Problem::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            "The report version is stale",
        ));
    }
    let reason = input.reason.unwrap_or(ReportReason::parse(&current.0)?);
    let explanation = validate_explanation(reason, input.explanation.or(current.1))?;
    sqlx::query(
        "UPDATE content_reports SET reason = $2, explanation = $3, version = version + 1, updated_at = now() \
         WHERE id = $1",
    )
    .bind(report_id.to_bytes().to_vec())
    .bind(reason.as_str())
    .bind(explanation)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    load_report(&state.pool, principal.user_id, report_id)
        .await
        .map(Json)
}

fn validate_explanation(
    reason: ReportReason,
    explanation: Option<String>,
) -> Result<Option<String>, Problem> {
    let explanation = explanation.map(|value| value.trim().to_owned());
    if matches!(reason, ReportReason::Other) && explanation.as_deref().is_none_or(str::is_empty) {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "report_reason_required",
            "An explanation is required when reason is other",
        ));
    }
    if explanation.as_deref().is_some_and(|value| {
        value.is_empty() || value.graphemes(true).count() > 500 || value.len() > 8192
    }) {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "problem_validation_failed",
            "explanation must contain 1 to 500 graphemes and at most 8192 UTF-8 bytes",
        ));
    }
    Ok(explanation)
}

async fn load_unresolved_report(
    connection: &mut sqlx::PgConnection,
    reporter_id: UserId,
    post_id: PostId,
) -> Result<Option<ReportResponse>, Problem> {
    let row: Option<ReportRow> = sqlx::query_as(
        "SELECT r.id, r.target_post_id, e.target_kind, r.reason, r.explanation, r.state, r.version, \
         to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(r.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM content_reports r JOIN content_report_evidence e ON e.report_id = r.id \
         WHERE r.reporter_id = $1 AND r.target_post_id = $2 AND r.state IN ('received', 'inReview')",
    )
    .bind(reporter_id.to_bytes().to_vec())
    .bind(post_id.to_bytes().to_vec())
    .fetch_optional(connection)
    .await
    .map_err(internal_error)?;
    row.map(report_response).transpose()
}

async fn load_report(
    pool: &sqlx::PgPool,
    reporter_id: UserId,
    report_id: ReportId,
) -> Result<ReportResponse, Problem> {
    let row: ReportRow = sqlx::query_as(
        "SELECT r.id, r.target_post_id, e.target_kind, r.reason, r.explanation, r.state, r.version, \
         to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(r.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM content_reports r JOIN content_report_evidence e ON e.report_id = r.id \
         WHERE r.id = $1 AND r.reporter_id = $2",
    )
    .bind(report_id.to_bytes().to_vec())
    .bind(reporter_id.to_bytes().to_vec())
    .fetch_optional(pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(report_not_found)?;
    report_response(row)
}

fn report_response(row: ReportRow) -> Result<ReportResponse, Problem> {
    Ok(ReportResponse {
        id: report_id(&row.0)?,
        target_id: post_id(&row.1)?,
        target_type: row.2,
        reason: ReportReason::parse(&row.3)?,
        explanation: row.4,
        status: row.5,
        version: row.6,
        created_at: row.7,
        updated_at: row.8,
    })
}

fn report_id(bytes: &[u8]) -> Result<ReportId, Problem> {
    bytes
        .try_into()
        .map(ReportId::from_bytes)
        .map_err(|_| internal_error("invalid report ID"))
}

fn post_id(bytes: &[u8]) -> Result<PostId, Problem> {
    bytes
        .try_into()
        .map(PostId::from_bytes)
        .map_err(|_| internal_error("invalid post ID"))
}

fn report_target_not_visible() -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        "Post not found",
    )
}

fn report_not_found() -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        "Report not found",
    )
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(error = %error, "report operation failed");
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An internal error occurred",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::Role;
    use axum::response::IntoResponse;
    use miz_api::domain::SessionId;

    fn principal(user_id: UserId) -> Principal {
        Principal {
            user_id,
            session_id: SessionId::from_bytes(user_id.to_bytes()),
            role: Role::User,
        }
    }

    #[tokio::test]
    async fn reports_are_deduplicated_editable_and_keep_evidence() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![8; 32],
            operator_mfa_key: [8; 32],
        };
        let reporter = UserId::new().unwrap();
        let author = UserId::new().unwrap();
        for (id, name) in [(reporter, "Reporter"), (author, "Reported author")] {
            sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
                .bind(id.to_bytes().to_vec())
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        let target = PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'original evidence', 'public')",
        )
        .bind(target.to_bytes().to_vec())
        .bind(author.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let reporter_principal = principal(reporter);

        let (status, Json(created)) = create_report(
            State(state.clone()),
            Extension(reporter_principal),
            Path(target),
            Json(CreateReportRequest {
                reason: ReportReason::Spam,
                explanation: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let (status, Json(repeated)) = create_report(
            State(state.clone()),
            Extension(reporter_principal),
            Path(target),
            Json(CreateReportRequest {
                reason: ReportReason::Harassment,
                explanation: Some("ignored retry".to_owned()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(repeated.id, created.id);

        sqlx::query("UPDATE posts SET content = 'edited later', version = 2 WHERE id = $1")
            .bind(target.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let evidence: String =
            sqlx::query_scalar("SELECT content FROM content_report_evidence WHERE report_id = $1")
                .bind(created.id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(evidence, "original evidence");

        let mut headers = HeaderMap::new();
        headers.insert("if-match", "\"1\"".parse().unwrap());
        let Json(updated) = update_report(
            State(state.clone()),
            Extension(reporter_principal),
            Path(created.id),
            headers,
            Json(UpdateReportRequest {
                reason: Some(ReportReason::Other),
                explanation: Some("additional context".to_owned()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.version, 2);
        let mut stale_headers = HeaderMap::new();
        stale_headers.insert("if-match", "\"1\"".parse().unwrap());
        let stale = update_report(
            State(state.clone()),
            Extension(reporter_principal),
            Path(created.id),
            stale_headers,
            Json(UpdateReportRequest {
                reason: Some(ReportReason::Spam),
                explanation: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(
            stale.into_response().status(),
            StatusCode::PRECONDITION_FAILED
        );
        let hidden_report = get_report(
            State(state.clone()),
            Extension(principal(author)),
            Path(created.id),
        )
        .await
        .unwrap_err();
        assert_eq!(
            hidden_report.into_response().status(),
            StatusCode::NOT_FOUND
        );

        for _ in 0..19 {
            let limited_target = PostId::new().unwrap();
            sqlx::query(
                "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'rate target', 'public')",
            )
            .bind(limited_target.to_bytes().to_vec())
            .bind(author.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO content_reports (id, reporter_id, target_post_id, reason) VALUES ($1, $2, $3, 'spam')",
            )
            .bind(ReportId::new().unwrap().to_bytes().to_vec())
            .bind(reporter.to_bytes().to_vec())
            .bind(limited_target.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        }
        let overflow_target = PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'overflow target', 'public')",
        )
        .bind(overflow_target.to_bytes().to_vec())
        .bind(author.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let limited = create_report(
            State(state.clone()),
            Extension(reporter_principal),
            Path(overflow_target),
            Json(CreateReportRequest {
                reason: ReportReason::Spam,
                explanation: None,
            }),
        )
        .await
        .unwrap_err()
        .into_response();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(limited.headers().contains_key("retry-after"));

        let own = create_report(
            State(state),
            Extension(principal(author)),
            Path(target),
            Json(CreateReportRequest {
                reason: ReportReason::Spam,
                explanation: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(own.into_response().status(), StatusCode::BAD_REQUEST);
    }
}
