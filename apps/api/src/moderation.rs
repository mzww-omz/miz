use crate::{
    api::{Problem, parse_if_match},
    operators::{OperatorPrincipal, operator_permission_denied},
    security::SecurityState,
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use miz_api::domain::{
    OperatorPermission, PostId, ReportId, RequestId, UserId, operator_authorized,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportQueueQuery {
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewReportRequest {
    status: String,
    reason: String,
    #[serde(default)]
    remove_content: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReportResponse {
    id: ReportId,
    reporter_id: Option<UserId>,
    target_id: PostId,
    target_type: String,
    author_id: UserId,
    reason: String,
    explanation: Option<String>,
    status: String,
    version: i64,
    created_at: String,
    updated_at: String,
    evidence_content: String,
    target_version: i64,
    target_created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReportList {
    items: Vec<AdminReportResponse>,
}

type AdminReportRow = (
    Vec<u8>,
    Option<Vec<u8>>,
    Vec<u8>,
    String,
    Vec<u8>,
    String,
    Option<String>,
    String,
    i64,
    String,
    String,
    String,
    i64,
    String,
);

pub async fn list_reports(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Query(query): Query<ReportQueueQuery>,
) -> Result<Json<AdminReportList>, Problem> {
    require_permission(&principal, OperatorPermission::ReviewReportEvidence)?;
    let status = query.status.unwrap_or_else(|| "received".to_owned());
    validate_status(&status)?;
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(validation("limit must be between 1 and 100"));
    }
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let rows: Vec<AdminReportRow> = sqlx::query_as(&format!(
        "{} WHERE r.state = $1 ORDER BY r.created_at, r.id LIMIT $2",
        report_select()
    ))
    .bind(&status)
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await
    .map_err(internal_error)?;
    insert_audit(
        &mut transaction,
        principal.operator_id,
        "operatorReportQueueRead",
        None,
        None,
        None,
        None,
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(AdminReportList {
        items: rows
            .into_iter()
            .map(report_response)
            .collect::<Result<_, _>>()?,
    }))
}

pub async fn get_report(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(report_id): Path<ReportId>,
) -> Result<Json<AdminReportResponse>, Problem> {
    require_permission(&principal, OperatorPermission::ReviewReportEvidence)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let row: AdminReportRow = sqlx::query_as(&format!("{} WHERE r.id = $1", report_select()))
        .bind(report_id.to_bytes().to_vec())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(internal_error)?
        .ok_or_else(report_not_found)?;
    insert_audit(
        &mut transaction,
        principal.operator_id,
        "operatorReportRead",
        Some("report"),
        Some(report_id.to_bytes().to_vec()),
        None,
        Some(report_id),
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    report_response(row).map(Json)
}

pub async fn review_report(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(report_id): Path<ReportId>,
    headers: HeaderMap,
    Json(input): Json<ReviewReportRequest>,
) -> Result<Json<AdminReportResponse>, Problem> {
    require_permission(&principal, OperatorPermission::ReviewReportEvidence)?;
    let expected_version = parse_if_match(&headers)?;
    let reason = input.reason.trim();
    if reason.is_empty() || reason.len() > 8192 {
        return Err(validation("reason must contain 1 to 8192 UTF-8 bytes"));
    }
    validate_status(&input.status)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: (String, i64, Vec<u8>, Option<String>) = sqlx::query_as(
        "SELECT r.state, r.version, r.target_post_id, p.state \
         FROM content_reports r LEFT JOIN posts p ON p.id = r.target_post_id \
         WHERE r.id = $1 FOR UPDATE OF r",
    )
    .bind(report_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(report_not_found)?;
    if current.1 != expected_version {
        return Err(Problem::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            "The report version is stale",
        ));
    }
    if !valid_transition(&current.0, &input.status) {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "invalid_report_transition",
            "The report state transition is not allowed",
        ));
    }
    if input.remove_content {
        require_permission(&principal, OperatorPermission::RemoveContent)?;
        if input.status != "actioned" {
            return Err(validation("removeContent requires actioned status"));
        }
    }

    sqlx::query(
        "UPDATE content_reports SET state = $2, version = version + 1, updated_at = now() WHERE id = $1",
    )
    .bind(report_id.to_bytes().to_vec())
    .bind(&input.status)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if matches!(input.status.as_str(), "actioned" | "noAction") {
        sqlx::query(
            "UPDATE content_report_evidence SET retain_until = now() + INTERVAL '180 days' WHERE report_id = $1",
        )
        .bind(report_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }
    if input.remove_content {
        let next_post_state: String = sqlx::query_scalar(
            "UPDATE posts SET content = NULL, \
             state = CASE WHEN EXISTS (SELECT 1 FROM posts child WHERE child.reply_to_post_id = posts.id) THEN 'tombstone' ELSE 'deleted' END, \
             version = version + 1, deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND state = 'published' RETURNING state",
        )
        .bind(&current.2)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(internal_error)?
        .unwrap_or_else(|| current.3.clone().unwrap_or_else(|| "missing".to_owned()));
        let action_id = RequestId::new().map_err(internal_error)?;
        sqlx::query(
            "INSERT INTO moderation_actions \
             (id, actor_operator_id, report_id, action_type, target_type, target_id, reason, before_state, after_state) \
             VALUES ($1, $2, $3, 'removeContent', 'post', $4, $5, jsonb_build_object('state', $6::text), jsonb_build_object('state', $7::text))",
        )
        .bind(action_id.to_bytes().to_vec())
        .bind(principal.operator_id.to_bytes().to_vec())
        .bind(report_id.to_bytes().to_vec())
        .bind(&current.2)
        .bind(reason)
        .bind(current.3.as_deref().unwrap_or("missing"))
        .bind(&next_post_state)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }
    insert_audit_state(
        &mut transaction,
        principal.operator_id,
        "operatorReportStateChanged",
        Some("report"),
        Some(report_id.to_bytes().to_vec()),
        Some(reason),
        Some(report_id),
        Some(serde_json::json!({ "status": current.0 }).to_string()),
        Some(serde_json::json!({ "status": input.status }).to_string()),
    )
    .await?;
    let row: AdminReportRow = sqlx::query_as(&format!("{} WHERE r.id = $1", report_select()))
        .bind(report_id.to_bytes().to_vec())
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    report_response(row).map(Json)
}

fn report_select() -> &'static str {
    "SELECT r.id, r.reporter_id, r.target_post_id, e.target_kind, e.author_id, r.reason, r.explanation, r.state, r.version, \
     to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
     to_char(r.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
     e.content, e.target_version, to_char(e.target_created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
     FROM content_reports r JOIN content_report_evidence e ON e.report_id = r.id"
}

fn report_response(row: AdminReportRow) -> Result<AdminReportResponse, Problem> {
    Ok(AdminReportResponse {
        id: report_id(&row.0)?,
        reporter_id: row.1.as_deref().map(user_id).transpose()?,
        target_id: post_id(&row.2)?,
        target_type: row.3,
        author_id: user_id(&row.4)?,
        reason: row.5,
        explanation: row.6,
        status: row.7,
        version: row.8,
        created_at: row.9,
        updated_at: row.10,
        evidence_content: row.11,
        target_version: row.12,
        target_created_at: row.13,
    })
}

pub(crate) async fn insert_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operator_id: miz_api::domain::OperatorId,
    event_type: &str,
    target_type: Option<&str>,
    target_id: Option<Vec<u8>>,
    reason: Option<&str>,
    report_id: Option<ReportId>,
) -> Result<(), Problem> {
    insert_audit_state(
        transaction,
        operator_id,
        event_type,
        target_type,
        target_id,
        reason,
        report_id,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_audit_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operator_id: miz_api::domain::OperatorId,
    event_type: &str,
    target_type: Option<&str>,
    target_id: Option<Vec<u8>>,
    reason: Option<&str>,
    report_id: Option<ReportId>,
    before_state: Option<String>,
    after_state: Option<String>,
) -> Result<(), Problem> {
    let request_id = RequestId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO audit_log_entries \
         (actor_operator_id, event_type, target_type, target_id, reason, report_id, request_id, before_state, after_state) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb, $9::jsonb)",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(event_type)
    .bind(target_type)
    .bind(target_id)
    .bind(reason)
    .bind(report_id.map(|id| id.to_bytes().to_vec()))
    .bind(request_id.to_bytes().to_vec())
    .bind(before_state)
    .bind(after_state)
    .execute(&mut **transaction)
    .await
    .map_err(internal_error)?;
    Ok(())
}

pub(crate) fn require_permission(
    principal: &OperatorPrincipal,
    permission: OperatorPermission,
) -> Result<(), Problem> {
    if principal
        .roles
        .iter()
        .any(|role| operator_authorized(*role, permission))
    {
        Ok(())
    } else {
        Err(operator_permission_denied())
    }
}

fn validate_status(status: &str) -> Result<(), Problem> {
    if matches!(status, "received" | "inReview" | "actioned" | "noAction") {
        Ok(())
    } else {
        Err(validation("Invalid report status"))
    }
}

fn valid_transition(current: &str, next: &str) -> bool {
    matches!(
        (current, next),
        ("received", "inReview") | ("inReview", "actioned" | "noAction")
    )
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

fn user_id(bytes: &[u8]) -> Result<UserId, Problem> {
    bytes
        .try_into()
        .map(UserId::from_bytes)
        .map_err(|_| internal_error("invalid user ID"))
}

fn report_not_found() -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        "Report not found",
    )
}

fn validation(detail: &str) -> Problem {
    Problem::new(StatusCode::BAD_REQUEST, "problem_validation_failed", detail)
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(error = %error, "moderation operation failed");
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An internal error occurred",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operators::OperatorPrincipal;
    use axum::response::IntoResponse;
    use miz_api::domain::{OperatorId, OperatorRole, SessionId};

    #[tokio::test]
    async fn report_review_is_audited_authorized_and_sets_retention() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![5; 32],
            operator_mfa_key: [5; 32],
        };
        let reporter = UserId::new().unwrap();
        let author = UserId::new().unwrap();
        for id in [reporter, author] {
            sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Moderation user')")
                .bind(id.to_bytes().to_vec())
                .execute(&pool)
                .await
                .unwrap();
        }
        let post_id = PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'reported evidence', 'public')",
        )
        .bind(post_id.to_bytes().to_vec())
        .bind(author.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let report_id = ReportId::new().unwrap();
        sqlx::query(
            "INSERT INTO content_reports (id, reporter_id, target_post_id, reason) VALUES ($1, $2, $3, 'spam')",
        )
        .bind(report_id.to_bytes().to_vec())
        .bind(reporter.to_bytes().to_vec())
        .bind(post_id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO content_report_evidence (report_id, target_kind, target_id, author_id, content, target_version, target_created_at) \
             VALUES ($1, 'post', $2, $3, 'reported evidence', 1, now())",
        )
        .bind(report_id.to_bytes().to_vec())
        .bind(post_id.to_bytes().to_vec())
        .bind(author.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let operator_id = OperatorId::new().unwrap();
        sqlx::query(
            "INSERT INTO operator_accounts (id, username, normalized_username) VALUES ($1, $2, $2)",
        )
        .bind(operator_id.to_bytes().to_vec())
        .bind(operator_id.to_string().to_ascii_lowercase())
        .execute(&pool)
        .await
        .unwrap();
        let moderator = OperatorPrincipal {
            operator_id,
            session_id: SessionId::new().unwrap(),
            roles: vec![OperatorRole::Moderator],
            recent_mfa: true,
        };

        let mut first_headers = HeaderMap::new();
        first_headers.insert("if-match", "\"1\"".parse().unwrap());
        let Json(in_review) = review_report(
            State(state.clone()),
            Extension(moderator.clone()),
            Path(report_id),
            first_headers,
            Json(ReviewReportRequest {
                status: "inReview".to_owned(),
                reason: "Review started".to_owned(),
                remove_content: false,
            }),
        )
        .await
        .unwrap();
        assert_eq!(in_review.status, "inReview");

        let mut action_headers = HeaderMap::new();
        action_headers.insert("if-match", "\"2\"".parse().unwrap());
        let Json(actioned) = review_report(
            State(state.clone()),
            Extension(moderator),
            Path(report_id),
            action_headers,
            Json(ReviewReportRequest {
                status: "actioned".to_owned(),
                reason: "Confirmed spam".to_owned(),
                remove_content: true,
            }),
        )
        .await
        .unwrap();
        assert_eq!(actioned.status, "actioned");
        let post_state: String = sqlx::query_scalar("SELECT state FROM posts WHERE id = $1")
            .bind(post_id.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(post_state, "deleted");
        let retained: bool = sqlx::query_scalar(
            "SELECT retain_until > now() + INTERVAL '179 days' FROM content_report_evidence WHERE report_id = $1",
        )
        .bind(report_id.to_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(retained);
        let actions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM moderation_actions WHERE report_id = $1")
                .bind(report_id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(actions, 1);

        let denied = list_reports(
            State(state),
            Extension(OperatorPrincipal {
                operator_id,
                session_id: SessionId::new().unwrap(),
                roles: vec![OperatorRole::Auditor],
                recent_mfa: true,
            }),
            Query(ReportQueueQuery {
                status: None,
                limit: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.into_response().status(), StatusCode::FORBIDDEN);
    }
}
