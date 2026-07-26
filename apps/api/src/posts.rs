use crate::{
    api::{Problem, parse_if_match},
    authorization::{Action, Principal, Role, authorize},
    pagination::{self, PageQuery},
    security::SecurityState,
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use miz_api::domain::{PostContent, PostContentError, PostId, UserId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CREATE_POST_ENDPOINT: &str = "/api/v1/posts";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostContentRequest {
    pub(crate) content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostResponse {
    pub(crate) id: PostId,
    pub(crate) author_id: UserId,
    pub(crate) reply_to_post_id: Option<PostId>,
    pub(crate) content: Option<String>,
    pub(crate) effective_visibility: String,
    pub(crate) state: String,
    pub(crate) version: i64,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) edited_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostPage {
    pub(crate) items: Vec<PostResponse>,
    pub(crate) next_cursor: Option<String>,
}

pub(crate) type PostRow = (
    Vec<u8>,
    Vec<u8>,
    Option<Vec<u8>>,
    Option<String>,
    String,
    String,
    i64,
    String,
    String,
    Option<String>,
);

pub async fn create_post(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<PostContentRequest>,
) -> Result<(StatusCode, Json<PostResponse>), Problem> {
    create_content(state, principal, headers, input, None).await
}

pub async fn create_reply(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(parent_id): Path<PostId>,
    headers: HeaderMap,
    Json(input): Json<PostContentRequest>,
) -> Result<(StatusCode, Json<PostResponse>), Problem> {
    create_content(state, principal, headers, input, Some(parent_id)).await
}

async fn create_content(
    state: SecurityState,
    principal: Principal,
    headers: HeaderMap,
    input: PostContentRequest,
    parent_id: Option<PostId>,
) -> Result<(StatusCode, Json<PostResponse>), Problem> {
    let key = idempotency_key(&headers)?;
    let content = PostContent::parse(&input.content).map_err(content_problem)?;
    let endpoint = parent_id
        .map(|id| format!("/api/v1/posts/{id}/replies"))
        .unwrap_or_else(|| CREATE_POST_ENDPOINT.to_owned());
    let request_hash = Sha256::digest(content.as_str().as_bytes()).to_vec();
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;

    let reserved: Option<(Option<i32>, Option<String>)> = sqlx::query_as(
        "INSERT INTO idempotency_keys (user_id, endpoint, idempotency_key, request_hash, expires_at) \
         VALUES ($1, $2, $3, $4, now() + INTERVAL '24 hours') \
         ON CONFLICT (user_id, endpoint, idempotency_key) DO UPDATE \
         SET request_hash = EXCLUDED.request_hash, response_status = NULL, response_body = NULL, expires_at = EXCLUDED.expires_at, created_at = now() \
         WHERE idempotency_keys.expires_at <= now() \
         RETURNING response_status, response_body::text",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(&endpoint)
    .bind(key)
    .bind(&request_hash)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?;

    if reserved.is_none() {
        let existing: (Vec<u8>, Option<i32>, Option<String>) = sqlx::query_as(
            "SELECT request_hash, response_status, response_body::text FROM idempotency_keys \
             WHERE user_id = $1 AND endpoint = $2 AND idempotency_key = $3 FOR UPDATE",
        )
        .bind(principal.user_id.to_bytes().to_vec())
        .bind(&endpoint)
        .bind(key)
        .fetch_one(&mut *transaction)
        .await
        .map_err(internal_error)?;
        if existing.0 != request_hash {
            return Err(Problem::new(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "Idempotency-Key was already used with different content",
            ));
        }
        let response = existing
            .2
            .ok_or_else(|| internal_error("incomplete idempotency record"))?;
        transaction.commit().await.map_err(internal_error)?;
        return Ok((
            StatusCode::from_u16(existing.1.unwrap_or(201) as u16).unwrap_or(StatusCode::CREATED),
            Json(serde_json::from_str(&response).map_err(internal_error)?),
        ));
    }

    let author_visibility: String = sqlx::query_scalar(
        "SELECT CASE privacy WHEN 'public' THEN 'public' ELSE 'followers' END FROM users WHERE id = $1 AND status = 'active'",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let parent_visibility = match parent_id {
        Some(parent_id) => {
            Some(load_reply_parent(&mut transaction, parent_id, principal, false).await?)
        }
        None => None,
    };
    let visibility =
        if author_visibility == "followers" || parent_visibility.as_deref() == Some("followers") {
            "followers"
        } else {
            "public"
        };
    let post_id = PostId::new().map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO posts (id, author_id, reply_to_post_id, content, effective_visibility) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(post_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(parent_id.map(|id| id.to_bytes().to_vec()))
    .bind(content.as_str())
    .bind(visibility)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    let response = load_post(&mut transaction, post_id, principal).await?;
    let response_json = serde_json::to_string(&response).map_err(internal_error)?;
    sqlx::query(
        "UPDATE idempotency_keys SET response_status = 201, response_body = $4::jsonb \
         WHERE user_id = $1 AND endpoint = $2 AND idempotency_key = $3",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(&endpoint)
    .bind(key)
    .bind(response_json)
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn get_post(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(post_id): Path<PostId>,
) -> Result<Json<PostResponse>, Problem> {
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let post = load_post(&mut transaction, post_id, principal).await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(post))
}

pub async fn list_replies(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(parent_id): Path<PostId>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PostPage>, Problem> {
    let limit = pagination::page_limit(query.limit)?;
    let scope = format!("replies:{parent_id}");
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| pagination::decode(&state.cursor_signing_key, &scope, value))
        .transpose()?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    load_reply_parent(&mut transaction, parent_id, principal, true).await?;
    let rows: Vec<PostRow> = sqlx::query_as(
        "SELECT p.id, p.author_id, p.reply_to_post_id, p.content, p.effective_visibility, p.state, p.version, \
         to_char(p.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(p.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         CASE WHEN p.edited_at IS NULL THEN NULL ELSE to_char(p.edited_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') END \
         FROM posts p JOIN users author ON author.id = p.author_id AND author.status = 'active' \
         WHERE p.reply_to_post_id = $1 AND p.state IN ('published', 'tombstone') \
         AND (author.privacy = 'public' OR p.author_id = $2 \
              OR EXISTS (SELECT 1 FROM follow_relationships f WHERE f.follower_id = $2 AND f.followee_id = p.author_id AND f.status = 'accepted') \
              OR $3) \
         AND ($4::timestamptz IS NULL OR (p.created_at, p.id) > ($4::timestamptz, $5)) \
         ORDER BY p.created_at ASC, p.id ASC LIMIT $6",
    )
    .bind(parent_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(is_privileged(principal))
    .bind(cursor.as_ref().map(|cursor| cursor.created_at.as_str()))
    .bind(cursor.as_ref().map(|cursor| cursor.id.to_bytes().to_vec()))
    .bind(limit + 1)
    .fetch_all(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    let has_more = rows.len() as i64 > limit;
    let items = rows
        .into_iter()
        .take(limit as usize)
        .map(post_response)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = if has_more {
        items
            .last()
            .map(|post| {
                pagination::encode(
                    &state.cursor_signing_key,
                    &scope,
                    post.created_at.clone(),
                    post.id,
                )
            })
            .transpose()?
    } else {
        None
    };
    Ok(Json(PostPage { items, next_cursor }))
}

pub async fn update_post(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(post_id): Path<PostId>,
    headers: HeaderMap,
    Json(input): Json<PostContentRequest>,
) -> Result<Json<PostResponse>, Problem> {
    let expected_version = parse_if_match(&headers)?;
    let content = PostContent::parse(&input.content).map_err(content_problem)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: (Vec<u8>, String, i64, bool) = sqlx::query_as(
        "SELECT author_id, content, version, created_at > now() - INTERVAL '15 minutes' \
         FROM posts WHERE id = $1 AND state = 'published' FOR UPDATE",
    )
    .bind(post_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    let owner_id = user_id(&current.0)?;
    if !authorize(principal, Action::Mutate, owner_id, false) {
        return Err(not_found());
    }
    if current.2 != expected_version {
        return Err(Problem::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            "The post was changed by another request",
        ));
    }
    if !current.3 {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "edit_window_expired",
            "Posts can only be edited for 15 minutes after creation",
        ));
    }
    if current.1 != content.as_str() {
        sqlx::query(
            "INSERT INTO post_edit_history (post_id, editor_id, previous_content, new_content) VALUES ($1, $2, $3, $4)",
        )
        .bind(post_id.to_bytes().to_vec())
        .bind(principal.user_id.to_bytes().to_vec())
        .bind(&current.1)
        .bind(content.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
        sqlx::query(
            "UPDATE posts SET content = $2, version = version + 1, edited_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(post_id.to_bytes().to_vec())
        .bind(content.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    }
    let post = load_post(&mut transaction, post_id, principal).await?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(post))
}

pub async fn delete_post(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(post_id): Path<PostId>,
    headers: HeaderMap,
) -> Result<Response, Problem> {
    let expected_version = parse_if_match(&headers)?;
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: (Vec<u8>, i64) = sqlx::query_as(
        "SELECT author_id, version FROM posts WHERE id = $1 AND state = 'published' FOR UPDATE",
    )
    .bind(post_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    if !authorize(principal, Action::Mutate, user_id(&current.0)?, false) {
        return Err(not_found());
    }
    if current.1 != expected_version {
        return Err(Problem::new(
            StatusCode::PRECONDITION_FAILED,
            "version_conflict",
            "The post was changed by another request",
        ));
    }
    let has_replies: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM posts WHERE reply_to_post_id = $1)")
            .bind(post_id.to_bytes().to_vec())
            .fetch_one(&mut *transaction)
            .await
            .map_err(internal_error)?;
    sqlx::query(
        "UPDATE posts SET content = NULL, state = $2, version = version + 1, deleted_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(post_id.to_bytes().to_vec())
    .bind(if has_replies { "tombstone" } else { "deleted" })
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query("DELETE FROM post_mentions WHERE post_id = $1")
        .bind(post_id.to_bytes().to_vec())
        .execute(&mut *transaction)
        .await
        .map_err(internal_error)?;
    sqlx::query(
        "INSERT INTO security_audit_log (actor_user_id, action, resource_type, resource_id) VALUES ($1, 'post.deleted', 'post', $2)",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(post_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn load_post(
    connection: &mut sqlx::PgConnection,
    post_id: PostId,
    principal: Principal,
) -> Result<PostResponse, Problem> {
    let row: PostRow = sqlx::query_as(
        "SELECT p.id, p.author_id, p.reply_to_post_id, p.content, p.effective_visibility, p.state, p.version, \
         to_char(p.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(p.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         CASE WHEN p.edited_at IS NULL THEN NULL ELSE to_char(p.edited_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') END \
         FROM posts p JOIN users author ON author.id = p.author_id AND author.status = 'active' \
         WHERE p.id = $1 AND p.state IN ('published', 'tombstone') \
         AND ((author.privacy = 'public' AND (p.reply_to_post_id IS NOT NULL OR p.effective_visibility = 'public')) OR p.author_id = $2 \
              OR EXISTS (SELECT 1 FROM follow_relationships f WHERE f.follower_id = $2 AND f.followee_id = p.author_id AND f.status = 'accepted') \
              OR $3) \
         AND (p.reply_to_post_id IS NULL OR EXISTS ( \
             SELECT 1 FROM posts parent JOIN users parent_author ON parent_author.id = parent.author_id AND parent_author.status = 'active' \
             WHERE parent.id = p.reply_to_post_id AND parent.state IN ('published', 'tombstone') \
             AND ((parent.effective_visibility = 'public' AND parent_author.privacy = 'public') OR parent.author_id = $2 \
                  OR EXISTS (SELECT 1 FROM follow_relationships f WHERE f.follower_id = $2 AND f.followee_id = parent.author_id AND f.status = 'accepted') \
                  OR $3)))",
    )
    .bind(post_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(is_privileged(principal))
    .fetch_optional(connection)
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    post_response(row)
}

async fn load_reply_parent(
    connection: &mut sqlx::PgConnection,
    parent_id: PostId,
    principal: Principal,
    allow_tombstone: bool,
) -> Result<String, Problem> {
    sqlx::query_scalar(
        "SELECT p.effective_visibility FROM posts p \
         JOIN users author ON author.id = p.author_id AND author.status = 'active' \
         WHERE p.id = $1 AND p.reply_to_post_id IS NULL \
         AND (p.state = 'published' OR ($4 AND p.state = 'tombstone')) \
         AND ((p.effective_visibility = 'public' AND author.privacy = 'public') OR p.author_id = $2 \
              OR EXISTS (SELECT 1 FROM follow_relationships f WHERE f.follower_id = $2 AND f.followee_id = p.author_id AND f.status = 'accepted') \
              OR $3) FOR SHARE OF p",
    )
    .bind(parent_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(is_privileged(principal))
    .bind(allow_tombstone)
    .fetch_optional(connection)
    .await
    .map_err(internal_error)?
    .ok_or_else(|| {
        Problem::new(
            StatusCode::NOT_FOUND,
            "parent_not_visible",
            "Parent post is not available",
        )
    })
}

pub(crate) fn post_response(row: PostRow) -> Result<PostResponse, Problem> {
    Ok(PostResponse {
        id: post_id_from_bytes(&row.0)?,
        author_id: user_id(&row.1)?,
        reply_to_post_id: row.2.as_deref().map(post_id_from_bytes).transpose()?,
        content: row.3,
        effective_visibility: row.4,
        state: row.5,
        version: row.6,
        created_at: row.7,
        updated_at: row.8,
        edited_at: row.9,
    })
}

fn is_privileged(principal: Principal) -> bool {
    matches!(principal.role, Role::Moderator | Role::Administrator)
}

fn idempotency_key(headers: &HeaderMap) -> Result<&str, Problem> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or_else(|| {
            Problem::new(
                StatusCode::PRECONDITION_REQUIRED,
                "idempotency_required",
                "Idempotency-Key containing 1 to 255 bytes is required",
            )
        })
}

fn content_problem(error: PostContentError) -> Problem {
    let (code, detail) = match error {
        PostContentError::Empty => ("content_empty", "content must not be blank"),
        PostContentError::TooLong => (
            "content_too_long",
            "content must contain at most 500 grapheme clusters",
        ),
        PostContentError::TooLarge => (
            "content_too_large",
            "content must contain at most 8192 bytes",
        ),
    };
    Problem::new(StatusCode::BAD_REQUEST, code, detail)
}

fn user_id(bytes: &[u8]) -> Result<UserId, Problem> {
    bytes
        .try_into()
        .map(UserId::from_bytes)
        .map_err(|_| internal_error("invalid user ID"))
}

fn post_id_from_bytes(bytes: &[u8]) -> Result<PostId, Problem> {
    bytes
        .try_into()
        .map(PostId::from_bytes)
        .map_err(|_| internal_error("invalid post ID"))
}

fn not_found() -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        "Post not found",
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
    use crate::authorization::Role;
    use axum::http::header;

    #[tokio::test]
    async fn create_edit_and_delete_enforce_content_invariants() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![7; 32],
        };
        let author = UserId::new().unwrap();
        let stranger = UserId::new().unwrap();
        for (id, name) in [(author, "Author"), (stranger, "Stranger")] {
            sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
                .bind(id.to_bytes().to_vec())
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        let principal = Principal {
            user_id: author,
            session_id: miz_api::domain::SessionId::new().unwrap(),
            role: Role::User,
        };
        let mut create_headers = HeaderMap::new();
        create_headers.insert("idempotency-key", "post-test-key".parse().unwrap());
        let (_, Json(created)) = create_post(
            State(state.clone()),
            Extension(principal),
            create_headers.clone(),
            Json(PostContentRequest {
                content: "  first\nsecond  ".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(created.content.as_deref(), Some("first\nsecond"));

        let (_, Json(replayed)) = create_post(
            State(state.clone()),
            Extension(principal),
            create_headers.clone(),
            Json(PostContentRequest {
                content: "first\nsecond".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(replayed.id, created.id);
        let conflict = create_post(
            State(state.clone()),
            Extension(principal),
            create_headers,
            Json(PostContentRequest {
                content: "different".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.into_response().status(), StatusCode::CONFLICT);

        let mut version_one = HeaderMap::new();
        version_one.insert(header::IF_MATCH, "\"1\"".parse().unwrap());
        let Json(edited) = update_post(
            State(state.clone()),
            Extension(principal),
            Path(created.id),
            version_one.clone(),
            Json(PostContentRequest {
                content: "edited".to_owned(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(edited.version, 2);
        assert!(edited.edited_at.is_some());
        let history: i64 =
            sqlx::query_scalar("SELECT count(*) FROM post_edit_history WHERE post_id = $1")
                .bind(created.id.to_bytes().to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(history, 1);

        let stale = update_post(
            State(state.clone()),
            Extension(principal),
            Path(created.id),
            version_one.clone(),
            Json(PostContentRequest {
                content: "stale edit".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(
            stale.into_response().status(),
            StatusCode::PRECONDITION_FAILED
        );
        sqlx::query("UPDATE posts SET created_at = now() - INTERVAL '16 minutes' WHERE id = $1")
            .bind(created.id.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let mut version_two = HeaderMap::new();
        version_two.insert(header::IF_MATCH, "\"2\"".parse().unwrap());
        let expired = update_post(
            State(state.clone()),
            Extension(principal),
            Path(created.id),
            version_two.clone(),
            Json(PostContentRequest {
                content: "late edit".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(expired.into_response().status(), StatusCode::CONFLICT);

        let stranger_principal = Principal {
            user_id: stranger,
            session_id: miz_api::domain::SessionId::new().unwrap(),
            role: Role::User,
        };
        let denied = delete_post(
            State(state.clone()),
            Extension(stranger_principal),
            Path(created.id),
            version_one,
        )
        .await
        .unwrap_err();
        assert_eq!(denied.into_response().status(), StatusCode::NOT_FOUND);

        assert_eq!(
            delete_post(
                State(state.clone()),
                Extension(principal),
                Path(created.id),
                version_two,
            )
            .await
            .unwrap()
            .status(),
            StatusCode::NO_CONTENT
        );
        let missing = get_post(State(state.clone()), Extension(principal), Path(created.id))
            .await
            .unwrap_err();
        assert_eq!(missing.into_response().status(), StatusCode::NOT_FOUND);

        let mut tombstone_headers = HeaderMap::new();
        tombstone_headers.insert("idempotency-key", "tombstone-test-key".parse().unwrap());
        let (_, Json(parent)) = create_post(
            State(state.clone()),
            Extension(principal),
            tombstone_headers,
            Json(PostContentRequest {
                content: "parent".to_owned(),
            }),
        )
        .await
        .unwrap();
        let child_id = PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, reply_to_post_id, content, effective_visibility) VALUES ($1, $2, $3, 'child', 'public')",
        )
        .bind(child_id.to_bytes().to_vec())
        .bind(stranger.to_bytes().to_vec())
        .bind(parent.id.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            delete_post(
                State(state.clone()),
                Extension(principal),
                Path(parent.id),
                {
                    let mut headers = HeaderMap::new();
                    headers.insert(header::IF_MATCH, "\"1\"".parse().unwrap());
                    headers
                },
            )
            .await
            .unwrap()
            .status(),
            StatusCode::NO_CONTENT
        );
        let Json(tombstone) = get_post(State(state), Extension(principal), Path(parent.id))
            .await
            .unwrap();
        assert_eq!(tombstone.state, "tombstone");
        assert!(tombstone.content.is_none());
    }
}
