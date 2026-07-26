use crate::{
    api::{Problem, parse_if_match},
    moderation::{insert_audit, insert_audit_state, require_permission},
    operators::{OperatorPrincipal, normalize_username, parse_role, require_recent_mfa, role_name},
    registration::require_same_origin,
    security::{self, SecurityState},
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use miz_api::{
    domain::{
        AppealId, Handle, OperatorId, OperatorPermission, OperatorRole, ReportId, RequestId,
        RestrictionId, UserId,
    },
    operator_security,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateOperatorRequest {
    username: String,
    password: String,
    roles: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorEnrollmentResponse {
    id: OperatorId,
    username: String,
    roles: Vec<String>,
    enrollment_token: String,
    provisioning_uri: String,
    recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplaceRolesRequest {
    roles: Vec<String>,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RestrictionRequest {
    kind: String,
    feature: Option<String>,
    expires_at: Option<String>,
    reason: String,
    report_id: Option<ReportId>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BasicAccountResponse {
    id: UserId,
    handle: String,
    display_name: String,
    status: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictionResponse {
    id: RestrictionId,
    action_id: RequestId,
    user_id: UserId,
    kind: String,
    feature: Option<String>,
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAppealRequest {
    username: Handle,
    password: String,
    action_id: RequestId,
    explanation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewAppealRequest {
    status: String,
    reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppealResponse {
    id: AppealId,
    action_id: RequestId,
    status: String,
    version: i64,
    created_at: String,
    reviewed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppealQueueQuery {
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminAppealResponse {
    id: AppealId,
    action_id: RequestId,
    appellant_user_id: UserId,
    explanation: String,
    status: String,
    version: i64,
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditQuery {
    target_type: String,
    target_id: RequestId,
    from: String,
    to: String,
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntryResponse {
    id: i64,
    actor_operator_id: Option<OperatorId>,
    event_type: String,
    target_type: String,
    target_id: RequestId,
    reason: Option<String>,
    before_state: Option<serde_json::Value>,
    after_state: Option<serde_json::Value>,
    request_id: Option<RequestId>,
    report_id: Option<ReportId>,
    created_at: String,
}

type AdminAppealRow = (Vec<u8>, Vec<u8>, Vec<u8>, String, String, i64, String);

pub async fn create_operator(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Json(input): Json<CreateOperatorRequest>,
) -> Result<(StatusCode, Json<OperatorEnrollmentResponse>), Problem> {
    require_permission(&principal, OperatorPermission::ManageOperatorRoles)?;
    require_recent_mfa(&principal)?;
    let username = normalize_username(&input.username)?;
    let roles = roles(&input.roles)?;
    let password_hash =
        tokio::task::spawn_blocking(move || operator_security::hash_password(input.password))
            .await
            .map_err(internal_error)?
            .map_err(validation)?;
    let operator_id = OperatorId::new().map_err(internal_error)?;
    let secret = operator_security::generate_totp_secret().map_err(internal_error)?;
    let (encrypted_secret, nonce) =
        operator_security::encrypt_totp_secret(&state.operator_mfa_key, &secret)
            .map_err(internal_error)?;
    let recovery_codes = operator_security::generate_recovery_codes().map_err(internal_error)?;
    let mut token = [0_u8; 32];
    getrandom::fill(&mut token).map_err(internal_error)?;
    let enrollment_token = URL_SAFE_NO_PAD.encode(token);
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO operator_accounts (id, username, normalized_username, status) VALUES ($1, $2, $2, 'pending')",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(&username)
    .execute(&mut *transaction)
    .await
    .map_err(map_operator_conflict)?;
    sqlx::query("INSERT INTO operator_credentials (operator_id, password_hash) VALUES ($1, $2)")
        .bind(operator_id.to_bytes().to_vec())
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO operator_mfa_factors (operator_id, encrypted_totp_secret, encryption_nonce) VALUES ($1, $2, $3)",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(encrypted_secret)
    .bind(nonce.to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    for role in &roles {
        sqlx::query("INSERT INTO operator_role_assignments (operator_id, role) VALUES ($1, $2)")
            .bind(operator_id.to_bytes().to_vec())
            .bind(role_name(role))
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
    }
    for code in &recovery_codes {
        sqlx::query("INSERT INTO operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)")
            .bind(operator_id.to_bytes().to_vec())
            .bind(operator_security::recovery_code_hash(code).to_vec())
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
    }
    sqlx::query(
        "INSERT INTO operator_mfa_enrollment_challenges (token_hash, operator_id, expires_at) VALUES ($1, $2, now() + INTERVAL '15 minutes')",
    )
    .bind(security::hash(enrollment_token.as_bytes()).to_vec())
    .bind(operator_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    insert_audit_state(
        &mut transaction,
        principal.operator_id,
        if roles.contains(&OperatorRole::Administrator) {
            "highPrivilegeRoleGrant"
        } else {
            "operatorCreated"
        },
        Some("operator"),
        Some(operator_id.to_bytes().to_vec()),
        Some("Operator enrollment created"),
        None,
        None,
        Some(json!({ "roles": roles.iter().map(role_name).collect::<Vec<_>>() }).to_string()),
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    let encoded_username: String =
        url::form_urlencoded::byte_serialize(username.as_bytes()).collect();
    Ok((
        StatusCode::CREATED,
        Json(OperatorEnrollmentResponse {
            id: operator_id,
            username,
            roles: roles.iter().map(role_name).map(str::to_owned).collect(),
            enrollment_token,
            provisioning_uri: format!(
                "otpauth://totp/MIZ:{encoded_username}?secret={}&issuer=MIZ&algorithm=SHA1&digits=6&period=30",
                operator_security::base32_secret(&secret)
            ),
            recovery_codes,
        }),
    ))
}

pub async fn replace_operator_roles(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(operator_id): Path<OperatorId>,
    Json(input): Json<ReplaceRolesRequest>,
) -> Result<StatusCode, Problem> {
    require_permission(&principal, OperatorPermission::ManageOperatorRoles)?;
    require_recent_mfa(&principal)?;
    let reason = required_reason(&input.reason)?;
    let roles = roles(&input.roles)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: Vec<String> = sqlx::query_scalar(
        "SELECT role FROM operator_role_assignments WHERE operator_id = $1 ORDER BY role FOR UPDATE",
    )
    .bind(operator_id.to_bytes().to_vec())
    .fetch_all(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if current.is_empty() {
        return Err(resource_not_found("Operator not found"));
    }
    if principal.operator_id == operator_id
        && current.iter().any(|role| role == "administrator")
        && !roles.contains(&OperatorRole::Administrator)
    {
        let administrators: i64 = sqlx::query_scalar(
            "SELECT count(DISTINCT operator_id) FROM operator_role_assignments WHERE role = 'administrator'",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?;
        if administrators <= 1 {
            return Err(Problem::new(
                StatusCode::CONFLICT,
                "invalid_state_transition",
                "The last administrator role cannot be removed",
            ));
        }
    }
    sqlx::query("DELETE FROM operator_role_assignments WHERE operator_id = $1")
        .bind(operator_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    for role in &roles {
        sqlx::query("INSERT INTO operator_role_assignments (operator_id, role) VALUES ($1, $2)")
            .bind(operator_id.to_bytes().to_vec())
            .bind(role_name(role))
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
    }
    let action_id = RequestId::new().map_err(internal_error)?;
    let before_state = json!({ "roles": current }).to_string();
    let after_state =
        json!({ "roles": roles.iter().map(role_name).collect::<Vec<_>>() }).to_string();
    sqlx::query(
        "INSERT INTO moderation_actions (id, actor_operator_id, action_type, target_type, target_id, reason, before_state, after_state) \
         VALUES ($1, $2, 'roleChange', 'operator', $3, $4, $5::jsonb, $6::jsonb)",
    )
    .bind(action_id.to_bytes().to_vec())
    .bind(principal.operator_id.to_bytes().to_vec())
    .bind(operator_id.to_bytes().to_vec())
    .bind(reason)
    .bind(&before_state)
    .bind(&after_state)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    insert_audit_state(
        &mut transaction,
        principal.operator_id,
        "highPrivilegeRoleChange",
        Some("operator"),
        Some(operator_id.to_bytes().to_vec()),
        Some(reason),
        None,
        Some(before_state),
        Some(after_state),
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_basic_account(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(user_id): Path<UserId>,
) -> Result<Json<BasicAccountResponse>, Problem> {
    require_permission(&principal, OperatorPermission::ReadBasicAccount)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let row: (Vec<u8>, String, String, String, String) = sqlx::query_as(
        "SELECT users.id, handle.value, users.display_name, users.status, \
         to_char(users.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM users JOIN handles handle ON handle.user_id = users.id AND handle.is_current \
         WHERE users.id = $1 AND users.status <> 'deleted'",
    )
    .bind(user_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| resource_not_found("User not found"))?;
    insert_audit(
        &mut transaction,
        principal.operator_id,
        "operatorBasicAccountRead",
        Some("user"),
        Some(user_id.to_bytes().to_vec()),
        None,
        None,
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(BasicAccountResponse {
        id: user_id,
        handle: row.1,
        display_name: row.2,
        status: row.3,
        created_at: row.4,
    }))
}

pub async fn apply_restriction(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(user_id): Path<UserId>,
    Json(input): Json<RestrictionRequest>,
) -> Result<(StatusCode, Json<RestrictionResponse>), Problem> {
    let permission = match input.kind.as_str() {
        "featureRestriction" => OperatorPermission::ApplyTemporaryRestriction,
        "temporarySuspension" => OperatorPermission::TemporarilySuspendAccount,
        "permanentSuspension" => OperatorPermission::PermanentlySuspendAccount,
        _ => return Err(validation("Invalid restriction kind")),
    };
    require_permission(&principal, permission)?;
    if input.kind == "permanentSuspension" {
        require_recent_mfa(&principal)?;
    }
    let reason = required_reason(&input.reason)?;
    if (input.kind == "featureRestriction") != input.feature.is_some() {
        return Err(validation(
            "feature is required only for featureRestriction",
        ));
    }
    if (input.kind == "permanentSuspension") != input.expires_at.is_none() {
        return Err(validation(
            "expiresAt is required only for temporary restrictions",
        ));
    }
    if let Some(expires_at) = &input.expires_at {
        let valid: bool = sqlx::query_scalar(
            "SELECT $1::timestamptz > now() AND $1::timestamptz <= now() + INTERVAL '30 days'",
        )
        .bind(expires_at)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| validation("expiresAt must be RFC 3339 within 30 days"))?;
        if !valid {
            return Err(validation("expiresAt must be in the next 30 days"));
        }
    }
    let action_id = RequestId::new().map_err(internal_error)?;
    let restriction_id = RestrictionId::new().map_err(internal_error)?;
    let action_type = match input.kind.as_str() {
        "featureRestriction" => "temporaryRestriction",
        "temporarySuspension" => "temporarySuspension",
        _ => "permanentSuspension",
    };
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let previous_status: String = sqlx::query_scalar(
        "SELECT status FROM users WHERE id = $1 AND status <> 'deleted' FOR UPDATE",
    )
    .bind(user_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| resource_not_found("User not found"))?;
    sqlx::query(
        "INSERT INTO moderation_actions (id, actor_operator_id, report_id, action_type, target_type, target_id, reason, before_state, after_state) \
         VALUES ($1, $2, $3, $4, 'user', $5, $6, jsonb_build_object('status', $7::text), jsonb_build_object('restriction', $8::text))",
    )
    .bind(action_id.to_bytes().to_vec())
    .bind(principal.operator_id.to_bytes().to_vec())
    .bind(input.report_id.map(|id| id.to_bytes().to_vec()))
    .bind(action_type)
    .bind(user_id.to_bytes().to_vec())
    .bind(reason)
    .bind(&previous_status)
    .bind(&input.kind)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO user_restrictions (id, user_id, action_id, kind, feature, reason, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)",
    )
    .bind(restriction_id.to_bytes().to_vec())
    .bind(user_id.to_bytes().to_vec())
    .bind(action_id.to_bytes().to_vec())
    .bind(&input.kind)
    .bind(&input.feature)
    .bind(reason)
    .bind(&input.expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if input.kind != "featureRestriction" {
        sqlx::query("UPDATE users SET status = 'suspended', updated_at = now() WHERE id = $1")
            .bind(user_id.to_bytes().to_vec())
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
        sqlx::query(
            "UPDATE sessions SET revoked_at = COALESCE(revoked_at, now()) WHERE user_id = $1",
        )
        .bind(user_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }
    insert_audit_state(
        &mut transaction,
        principal.operator_id,
        if input.kind == "permanentSuspension" {
            "highPrivilegePermanentSuspension"
        } else {
            "operatorRestrictionApplied"
        },
        Some("user"),
        Some(user_id.to_bytes().to_vec()),
        Some(reason),
        input.report_id,
        Some(json!({ "status": previous_status }).to_string()),
        Some(json!({ "restriction": input.kind }).to_string()),
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok((
        StatusCode::CREATED,
        Json(RestrictionResponse {
            id: restriction_id,
            action_id,
            user_id,
            kind: input.kind,
            feature: input.feature,
            expires_at: input.expires_at,
        }),
    ))
}

pub async fn create_appeal(
    State(state): State<SecurityState>,
    headers: axum::http::HeaderMap,
    Json(input): Json<CreateAppealRequest>,
) -> Result<(StatusCode, Json<AppealResponse>), Problem> {
    require_same_origin(&headers, &state.origin)?;
    let username = input.username.to_string();
    security::enforce_rate_limit(&state.pool, "moderation-appeal", &username, 5).await?;
    let credential: Option<(Vec<u8>, String)> = sqlx::query_as(
        "SELECT users.id, credential.password_hash FROM users \
         JOIN handles handle ON handle.user_id = users.id AND handle.is_current \
         JOIN password_credentials credential ON credential.user_id = users.id \
         WHERE handle.normalized = $1 AND users.status IN ('active', 'suspended')",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    let stored_hash = credential
        .as_ref()
        .map(|row| row.1.as_str())
        .unwrap_or_else(|| operator_security::dummy_password_hash());
    if !operator_security::verify_password(&input.password, stored_hash) {
        return Err(Problem::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Credentials are invalid",
        ));
    }
    let user_id = credential
        .and_then(|row| row.0.try_into().ok())
        .map(UserId::from_bytes)
        .ok_or_else(|| resource_not_found("Moderation action not found"))?;
    let explanation = required_reason(&input.explanation)?;
    let action_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM moderation_actions WHERE id = $1 AND target_type = 'user' AND target_id = $2)",
    )
    .bind(input.action_id.to_bytes().to_vec())
    .bind(user_id.to_bytes().to_vec())
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error)?;
    if !action_exists {
        return Err(resource_not_found("Moderation action not found"));
    }
    let appeal_id = AppealId::new().map_err(internal_error)?;
    let row: (Vec<u8>, Vec<u8>, String, i64, String, Option<String>) = sqlx::query_as(
        "INSERT INTO moderation_appeals (id, action_id, appellant_user_id, explanation) \
         VALUES ($1, $2, $3, $4) RETURNING id, action_id, state, version, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), NULL::text",
    )
    .bind(appeal_id.to_bytes().to_vec())
    .bind(input.action_id.to_bytes().to_vec())
    .bind(user_id.to_bytes().to_vec())
    .bind(explanation)
    .fetch_one(&state.pool)
    .await
    .map_err(|error| map_conflict(error, "appeal_already_exists"))?;
    Ok((StatusCode::CREATED, Json(appeal_response(row)?)))
}

pub async fn list_appeals(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Query(query): Query<AppealQueueQuery>,
) -> Result<Json<Vec<AdminAppealResponse>>, Problem> {
    require_permission(&principal, OperatorPermission::ReviewAppeal)?;
    let status = query.status.unwrap_or_else(|| "pending".to_owned());
    if !matches!(status.as_str(), "pending" | "upheld" | "overturned") {
        return Err(validation("Invalid appeal status"));
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(validation("limit must be between 1 and 100"));
    }
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let rows: Vec<AdminAppealRow> = sqlx::query_as(
        "SELECT id, action_id, appellant_user_id, explanation, state, version, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM moderation_appeals WHERE state = $1 ORDER BY created_at, id LIMIT $2",
    )
    .bind(status)
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await
    .map_err(internal_error)?;
    insert_audit(
        &mut transaction,
        principal.operator_id,
        "operatorAppealQueueRead",
        None,
        None,
        None,
        None,
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(AdminAppealResponse {
                id: appeal_id(&row.0)?,
                action_id: request_id(&row.1)?,
                appellant_user_id: user_id(&row.2)?,
                explanation: row.3,
                status: row.4,
                version: row.5,
                created_at: row.6,
            })
        })
        .collect::<Result<Vec<_>, Problem>>()
        .map(Json)
}

pub async fn review_appeal(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Path(appeal_id): Path<AppealId>,
    headers: axum::http::HeaderMap,
    Json(input): Json<ReviewAppealRequest>,
) -> Result<Json<AppealResponse>, Problem> {
    require_permission(&principal, OperatorPermission::ReviewAppeal)?;
    let version = parse_if_match(&headers)?;
    if !matches!(input.status.as_str(), "upheld" | "overturned") {
        return Err(validation("status must be upheld or overturned"));
    }
    let reason = required_reason(&input.reason)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: (Vec<u8>, String, i64, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT appeal.action_id, appeal.state, appeal.version, action.actor_operator_id, action.target_id \
         FROM moderation_appeals appeal JOIN moderation_actions action ON action.id = appeal.action_id \
         WHERE appeal.id = $1 FOR UPDATE OF appeal",
    )
    .bind(appeal_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| resource_not_found("Appeal not found"))?;
    if current.3 == principal.operator_id.to_bytes() {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "appeal_reviewer_conflict",
            "The original decision-maker cannot review this appeal",
        ));
    }
    if current.1 != "pending" {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "invalid_state_transition",
            "Appeal is no longer pending",
        ));
    }
    if current.2 != version {
        return Err(Problem::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            "The appeal version is stale",
        ));
    }
    let row: (Vec<u8>, Vec<u8>, String, i64, String, Option<String>) = sqlx::query_as(
        "UPDATE moderation_appeals SET state = $2, reviewer_operator_id = $3, review_reason = $4, \
         reviewed_at = now(), version = version + 1 WHERE id = $1 RETURNING id, action_id, state, version, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(reviewed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .bind(appeal_id.to_bytes().to_vec())
    .bind(&input.status)
    .bind(principal.operator_id.to_bytes().to_vec())
    .bind(reason)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    if input.status == "overturned" {
        sqlx::query("UPDATE user_restrictions SET revoked_at = now() WHERE action_id = $1")
            .bind(&current.0)
            .execute(&mut *transaction)
            .await
            .map_err(internal_error)?;
        sqlx::query(
            "UPDATE users SET status = 'active', updated_at = now() WHERE id = $1 AND status = 'suspended' \
             AND NOT EXISTS (SELECT 1 FROM user_restrictions restriction WHERE restriction.user_id = users.id \
               AND restriction.kind IN ('temporarySuspension', 'permanentSuspension') AND restriction.revoked_at IS NULL \
               AND (restriction.expires_at IS NULL OR restriction.expires_at > now()))",
        )
        .bind(&current.4)
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }
    insert_audit_state(
        &mut transaction,
        principal.operator_id,
        "operatorAppealReviewed",
        Some("appeal"),
        Some(appeal_id.to_bytes().to_vec()),
        Some(reason),
        None,
        Some(json!({ "status": current.1 }).to_string()),
        Some(json!({ "status": input.status }).to_string()),
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    appeal_response(row).map(Json)
}

pub async fn read_audit_log(
    State(state): State<SecurityState>,
    Extension(principal): Extension<OperatorPrincipal>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<AuditEntryResponse>>, Problem> {
    require_permission(&principal, OperatorPermission::ReadAuditLog)?;
    if !matches!(
        query.target_type.as_str(),
        "user" | "post" | "report" | "operator" | "appeal"
    ) {
        return Err(validation("Invalid audit targetType"));
    }
    let limit = query.limit.unwrap_or(100);
    if !(1..=100).contains(&limit) {
        return Err(validation("limit must be between 1 and 100"));
    }
    let range_valid: bool = sqlx::query_scalar(
        "SELECT $1::timestamptz <= $2::timestamptz \
         AND $2::timestamptz <= now() AND $2::timestamptz - $1::timestamptz <= INTERVAL '31 days'",
    )
    .bind(&query.from)
    .bind(&query.to)
    .fetch_one(&state.pool)
    .await
    .map_err(|_| validation("from and to must be a bounded RFC 3339 range"))?;
    if !range_valid {
        return Err(validation(
            "Audit range must end now or earlier and span at most 31 days",
        ));
    }
    type AuditRow = (
        i64,
        Option<Vec<u8>>,
        String,
        String,
        Vec<u8>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        String,
    );
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT id, actor_operator_id, event_type, target_type, target_id, reason, \
         before_state::text, after_state::text, request_id, report_id, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM audit_log_entries WHERE target_type = $1 AND target_id = $2 \
         AND created_at >= $3::timestamptz AND created_at <= $4::timestamptz \
         ORDER BY created_at DESC, id DESC LIMIT $5",
    )
    .bind(&query.target_type)
    .bind(query.target_id.to_bytes().to_vec())
    .bind(&query.from)
    .bind(&query.to)
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await
    .map_err(internal_error)?;
    insert_audit(
        &mut transaction,
        principal.operator_id,
        "operatorAuditRead",
        Some(&query.target_type),
        Some(query.target_id.to_bytes().to_vec()),
        None,
        None,
    )
    .await?;
    transaction.commit().await.map_err(internal_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(AuditEntryResponse {
                id: row.0,
                actor_operator_id: row.1.as_deref().map(operator_id).transpose()?,
                event_type: row.2,
                target_type: row.3,
                target_id: request_id(&row.4)?,
                reason: row.5,
                before_state: row
                    .6
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(internal_error)?,
                after_state: row
                    .7
                    .map(|value| serde_json::from_str(&value))
                    .transpose()
                    .map_err(internal_error)?,
                request_id: row.8.as_deref().map(request_id).transpose()?,
                report_id: row.9.as_deref().map(report_id).transpose()?,
                created_at: row.10,
            })
        })
        .collect::<Result<Vec<_>, Problem>>()
        .map(Json)
}

fn roles(values: &[String]) -> Result<Vec<OperatorRole>, Problem> {
    if values.is_empty() {
        return Err(validation("At least one operator role is required"));
    }
    if values.iter().any(|value| {
        !matches!(
            value.as_str(),
            "support" | "moderator" | "seniorModerator" | "administrator" | "auditor"
        )
    }) {
        return Err(validation("Invalid operator role"));
    }
    let parsed = values
        .iter()
        .map(|value| parse_role(value))
        .collect::<Result<Vec<_>, _>>()?;
    let unique: HashSet<_> = values.iter().collect();
    if unique.len() != values.len() {
        return Err(validation("Operator roles must be unique"));
    }
    Ok(parsed)
}

fn required_reason(value: &str) -> Result<&str, Problem> {
    let value = value.trim();
    if value.is_empty() || value.len() > 8192 {
        Err(validation("reason must contain 1 to 8192 UTF-8 bytes"))
    } else {
        Ok(value)
    }
}

fn appeal_response(
    row: (Vec<u8>, Vec<u8>, String, i64, String, Option<String>),
) -> Result<AppealResponse, Problem> {
    Ok(AppealResponse {
        id: appeal_id(&row.0)?,
        action_id: request_id(&row.1)?,
        status: row.2,
        version: row.3,
        created_at: row.4,
        reviewed_at: row.5,
    })
}

fn user_id(bytes: &[u8]) -> Result<UserId, Problem> {
    bytes
        .try_into()
        .map(UserId::from_bytes)
        .map_err(|_| internal_error("invalid user ID"))
}
fn operator_id(bytes: &[u8]) -> Result<OperatorId, Problem> {
    bytes
        .try_into()
        .map(OperatorId::from_bytes)
        .map_err(|_| internal_error("invalid operator ID"))
}
fn request_id(bytes: &[u8]) -> Result<RequestId, Problem> {
    bytes
        .try_into()
        .map(RequestId::from_bytes)
        .map_err(|_| internal_error("invalid object ID"))
}
fn report_id(bytes: &[u8]) -> Result<ReportId, Problem> {
    bytes
        .try_into()
        .map(ReportId::from_bytes)
        .map_err(|_| internal_error("invalid report ID"))
}
fn appeal_id(bytes: &[u8]) -> Result<AppealId, Problem> {
    bytes
        .try_into()
        .map(AppealId::from_bytes)
        .map_err(|_| internal_error("invalid appeal ID"))
}

fn validation(detail: impl std::fmt::Display) -> Problem {
    Problem::new(
        StatusCode::BAD_REQUEST,
        "problem_validation_failed",
        detail.to_string(),
    )
}
fn resource_not_found(detail: &str) -> Problem {
    Problem::new(StatusCode::NOT_FOUND, "resource_not_found", detail)
}
fn map_operator_conflict(error: sqlx::Error) -> Problem {
    map_conflict(error, "operator_username_unavailable")
}
fn map_conflict(error: sqlx::Error, code: &'static str) -> Problem {
    if matches!(&error, sqlx::Error::Database(database) if database.constraint().is_some()) {
        Problem::new(StatusCode::CONFLICT, code, "The resource already exists")
    } else {
        internal_error(error)
    }
}
fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(error = %error, "administration operation failed");
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An internal error occurred",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::HeaderMap, response::IntoResponse};
    use miz_api::domain::SessionId;

    async fn operator(pool: &sqlx::PgPool, role: &str) -> OperatorPrincipal {
        let id = OperatorId::new().unwrap();
        let username = id.to_string().to_ascii_lowercase();
        sqlx::query(
            "INSERT INTO operator_accounts (id, username, normalized_username) VALUES ($1, $2, $2)",
        )
        .bind(id.to_bytes().to_vec())
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO operator_role_assignments (operator_id, role) VALUES ($1, $2)")
            .bind(id.to_bytes().to_vec())
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
        OperatorPrincipal {
            operator_id: id,
            session_id: SessionId::new().unwrap(),
            roles: vec![parse_role(role).unwrap()],
            recent_mfa: true,
        }
    }

    #[tokio::test]
    async fn operator_enrollment_enforcement_appeal_and_audit_obey_rbac() {
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
        let administrator = operator(&pool, "administrator").await;
        let reviewer = operator(&pool, "seniorModerator").await;
        let auditor = operator(&pool, "auditor").await;

        let (_, Json(enrollment)) = create_operator(
            State(state.clone()),
            Extension(administrator.clone()),
            Json(CreateOperatorRequest {
                username: format!("mod.{}", &OperatorId::new().unwrap().to_string()[..12]),
                password: "correct horse battery staple".to_owned(),
                roles: vec!["moderator".to_owned()],
            }),
        )
        .await
        .unwrap();
        assert_eq!(enrollment.recovery_codes.len(), 10);
        let factor: (Vec<u8>, Vec<u8>) = sqlx::query_as(
            "SELECT encrypted_totp_secret, encryption_nonce FROM operator_mfa_factors WHERE operator_id = $1",
        )
        .bind(enrollment.id.to_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        let secret =
            operator_security::decrypt_totp_secret(&state.operator_mfa_key, &factor.0, &factor.1)
                .unwrap();
        let mut origin = HeaderMap::new();
        origin.insert("origin", "https://m1z.jp".parse().unwrap());
        crate::operators::enroll_mfa(
            State(state.clone()),
            origin.clone(),
            Json(crate::operators::OperatorEnrollmentRequest {
                enrollment_token: enrollment.enrollment_token,
                totp_code: operator_security::totp_code(
                    &secret,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ),
            }),
        )
        .await
        .unwrap();
        let enrolled_status: String =
            sqlx::query_scalar("SELECT status FROM operator_accounts WHERE id = $1")
                .bind(enrollment.id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(enrolled_status, "active");

        let user_id = UserId::new().unwrap();
        let username = user_id.to_string().to_ascii_lowercase();
        let password = "appealable password";
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Appealing user')")
            .bind(user_id.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO handles (value, normalized, user_id, is_current) VALUES ($1, $1, $2, true)",
        )
        .bind(&username)
        .bind(user_id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO password_credentials (user_id, password_hash) VALUES ($1, $2)")
            .bind(user_id.to_bytes().to_vec())
            .bind(operator_security::hash_password(password.to_owned()).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let (_, Json(restriction)) = apply_restriction(
            State(state.clone()),
            Extension(administrator.clone()),
            Path(user_id),
            Json(RestrictionRequest {
                kind: "permanentSuspension".to_owned(),
                feature: None,
                expires_at: None,
                reason: "Confirmed abuse".to_owned(),
                report_id: None,
            }),
        )
        .await
        .unwrap();
        let (_, Json(appeal)) = create_appeal(
            State(state.clone()),
            origin,
            Json(CreateAppealRequest {
                username: username.parse().unwrap(),
                password: password.to_owned(),
                action_id: restriction.action_id,
                explanation: "Please review this decision".to_owned(),
            }),
        )
        .await
        .unwrap();
        let mut if_match = HeaderMap::new();
        if_match.insert("if-match", "\"1\"".parse().unwrap());
        let conflict = review_appeal(
            State(state.clone()),
            Extension(administrator),
            Path(appeal.id),
            if_match.clone(),
            Json(ReviewAppealRequest {
                status: "overturned".to_owned(),
                reason: "Reconsidered".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.into_response().status(), StatusCode::CONFLICT);
        let Json(reviewed) = review_appeal(
            State(state.clone()),
            Extension(reviewer),
            Path(appeal.id),
            if_match,
            Json(ReviewAppealRequest {
                status: "overturned".to_owned(),
                reason: "Evidence did not support suspension".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(reviewed.status, "overturned");
        let status: String = sqlx::query_scalar("SELECT status FROM users WHERE id = $1")
            .bind(user_id.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "active");

        let (from, to): (String, String) = sqlx::query_as(
            "SELECT to_char((now() - INTERVAL '1 hour') AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
             to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let Json(entries) = read_audit_log(
            State(state),
            Extension(auditor),
            Query(AuditQuery {
                target_type: "user".to_owned(),
                target_id: RequestId::from_bytes(user_id.to_bytes()),
                from,
                to,
                limit: None,
            }),
        )
        .await
        .unwrap();
        assert!(!entries.is_empty());
    }
}
