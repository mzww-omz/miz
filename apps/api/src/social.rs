use crate::{
    api::Problem,
    authorization::{Principal, Role},
    pagination::{self, PageQuery},
    posts::{PostPage, PostRow, post_response},
    security::SecurityState,
};
use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use miz_api::domain::{FollowRelationshipId, UserId};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowRelationshipResponse {
    id: FollowRelationshipId,
    follower_id: UserId,
    followee_id: UserId,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowRelationshipList {
    items: Vec<FollowRelationshipResponse>,
    total: usize,
}

type RelationshipRow = (Vec<u8>, Vec<u8>, Vec<u8>, String, String, String);

pub async fn follow_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(target_id): Path<UserId>,
) -> Result<Json<FollowRelationshipResponse>, Problem> {
    if principal.user_id == target_id {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "cannot_follow_self",
            "A user cannot follow themselves",
        ));
    }
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let privacy: String = sqlx::query_scalar(
        "SELECT privacy FROM users WHERE id = $1 AND status = 'active' FOR SHARE",
    )
    .bind(target_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(target_not_visible)?;
    let status = if privacy == "public" {
        "accepted"
    } else {
        "pending"
    };
    let relationship_id = FollowRelationshipId::new().map_err(internal_error)?;
    let row: RelationshipRow = sqlx::query_as(
        "INSERT INTO follow_relationships (id, follower_id, followee_id, status) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (follower_id, followee_id) DO UPDATE SET \
         status = CASE WHEN follow_relationships.status IN ('pending', 'accepted') THEN follow_relationships.status ELSE EXCLUDED.status END, \
         updated_at = CASE WHEN follow_relationships.status IN ('pending', 'accepted') OR follow_relationships.status = EXCLUDED.status THEN follow_relationships.updated_at ELSE now() END \
         RETURNING id, follower_id, followee_id, status, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .bind(relationship_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(target_id.to_bytes().to_vec())
    .bind(status)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(Json(relationship_response(row)?))
}

pub async fn unfollow_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(target_id): Path<UserId>,
) -> Result<Response, Problem> {
    sqlx::query(
        "UPDATE follow_relationships SET \
         status = CASE status WHEN 'pending' THEN 'cancelled' WHEN 'accepted' THEN 'removed' ELSE status END, \
         updated_at = CASE WHEN status IN ('pending', 'accepted') THEN now() ELSE updated_at END \
         WHERE follower_id = $1 AND followee_id = $2",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(target_id.to_bytes().to_vec())
    .execute(&state.pool)
    .await
    .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn list_follow_requests(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<FollowRelationshipList>, Problem> {
    relationship_list(
        &state,
        "followee_id = $1 AND status = 'pending'",
        principal.user_id,
    )
    .await
    .map(Json)
}

pub async fn accept_follow_request(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(relationship_id): Path<FollowRelationshipId>,
) -> Result<Json<FollowRelationshipResponse>, Problem> {
    transition_request(&state, principal, relationship_id, "accepted")
        .await
        .map(Json)
}

pub async fn reject_follow_request(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(relationship_id): Path<FollowRelationshipId>,
) -> Result<Json<FollowRelationshipResponse>, Problem> {
    transition_request(&state, principal, relationship_id, "rejected")
        .await
        .map(Json)
}

pub async fn list_followers(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(user_id): Path<UserId>,
) -> Result<Json<FollowRelationshipList>, Problem> {
    ensure_relationships_visible(&state, principal, user_id).await?;
    relationship_list(&state, "followee_id = $1 AND status = 'accepted'", user_id)
        .await
        .map(Json)
}

pub async fn list_following(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(user_id): Path<UserId>,
) -> Result<Json<FollowRelationshipList>, Problem> {
    ensure_relationships_visible(&state, principal, user_id).await?;
    relationship_list(&state, "follower_id = $1 AND status = 'accepted'", user_id)
        .await
        .map(Json)
}

pub async fn home_timeline(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PostPage>, Problem> {
    let limit = pagination::page_limit(query.limit)?;
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| pagination::decode(&state.cursor_signing_key, "timeline", value))
        .transpose()?;
    let rows: Vec<PostRow> = sqlx::query_as(
        "SELECT p.id, p.author_id, p.reply_to_post_id, p.content, p.effective_visibility, p.state, p.version, \
         to_char(p.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(p.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         CASE WHEN p.edited_at IS NULL THEN NULL ELSE to_char(p.edited_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') END \
         FROM posts p JOIN users author ON author.id = p.author_id AND author.status = 'active' \
         WHERE p.reply_to_post_id IS NULL AND p.state = 'published' \
         AND (p.author_id = $1 OR EXISTS (SELECT 1 FROM follow_relationships f \
              WHERE f.follower_id = $1 AND f.followee_id = p.author_id AND f.status = 'accepted')) \
         AND ((p.effective_visibility = 'public' AND author.privacy = 'public') OR p.author_id = $1 \
              OR EXISTS (SELECT 1 FROM follow_relationships f WHERE f.follower_id = $1 AND f.followee_id = p.author_id AND f.status = 'accepted') \
              OR $2) \
         AND ($3::timestamptz IS NULL OR (p.created_at, p.id) < ($3::timestamptz, $4)) \
         ORDER BY p.created_at DESC, p.id DESC LIMIT $5",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(is_privileged(principal))
    .bind(cursor.as_ref().map(|cursor| cursor.created_at.as_str()))
    .bind(cursor.as_ref().map(|cursor| cursor.id.to_bytes().to_vec()))
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_error)?;
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
                    "timeline",
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

async fn transition_request(
    state: &SecurityState,
    principal: Principal,
    relationship_id: FollowRelationshipId,
    next_status: &str,
) -> Result<FollowRelationshipResponse, Problem> {
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    let current: (Vec<u8>, String) = sqlx::query_as(
        "SELECT followee_id, status FROM follow_relationships WHERE id = $1 FOR UPDATE",
    )
    .bind(relationship_id.to_bytes().to_vec())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal_error)?
    .ok_or_else(resource_not_found)?;
    if user_id(&current.0)? != principal.user_id {
        return Err(resource_not_found());
    }
    if current.1 != "pending" {
        return Err(Problem::new(
            StatusCode::CONFLICT,
            "invalid_state_transition",
            "Follow request is not pending",
        ));
    }
    let row: RelationshipRow = sqlx::query_as(
        "UPDATE follow_relationships SET status = $2, updated_at = now() WHERE id = $1 \
         RETURNING id, follower_id, followee_id, status, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')",
    )
    .bind(relationship_id.to_bytes().to_vec())
    .bind(next_status)
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    relationship_response(row)
}

// ponytail: relationship lists are unpaginated for MVP; add signed keyset cursors when response size becomes material.
async fn relationship_list(
    state: &SecurityState,
    condition: &str,
    user_id: UserId,
) -> Result<FollowRelationshipList, Problem> {
    let sql = format!(
        "SELECT id, follower_id, followee_id, status, \
         to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
         to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM follow_relationships WHERE {condition} ORDER BY created_at DESC, id DESC"
    );
    let rows: Vec<RelationshipRow> = sqlx::query_as(&sql)
        .bind(user_id.to_bytes().to_vec())
        .fetch_all(&state.pool)
        .await
        .map_err(internal_error)?;
    let items = rows
        .into_iter()
        .map(relationship_response)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FollowRelationshipList {
        total: items.len(),
        items,
    })
}

async fn ensure_relationships_visible(
    state: &SecurityState,
    principal: Principal,
    user_id: UserId,
) -> Result<(), Problem> {
    let visible: bool = sqlx::query_scalar(
        "SELECT privacy = 'public' OR id = $2 OR $3 OR EXISTS ( \
         SELECT 1 FROM follow_relationships f WHERE f.follower_id = $2 AND f.followee_id = users.id AND f.status = 'accepted') \
         FROM users WHERE id = $1 AND status = 'active'",
    )
    .bind(user_id.to_bytes().to_vec())
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(is_privileged(principal))
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?
    .ok_or_else(target_not_visible)?;
    if visible {
        Ok(())
    } else {
        Err(target_not_visible())
    }
}

fn relationship_response(row: RelationshipRow) -> Result<FollowRelationshipResponse, Problem> {
    Ok(FollowRelationshipResponse {
        id: relationship_id(&row.0)?,
        follower_id: user_id(&row.1)?,
        followee_id: user_id(&row.2)?,
        status: row.3,
        created_at: row.4,
        updated_at: row.5,
    })
}

fn is_privileged(principal: Principal) -> bool {
    matches!(principal.role, Role::Moderator | Role::Administrator)
}

fn relationship_id(bytes: &[u8]) -> Result<FollowRelationshipId, Problem> {
    bytes
        .try_into()
        .map(FollowRelationshipId::from_bytes)
        .map_err(|_| internal_error("invalid relationship ID"))
}

fn user_id(bytes: &[u8]) -> Result<UserId, Problem> {
    bytes
        .try_into()
        .map(UserId::from_bytes)
        .map_err(|_| internal_error("invalid user ID"))
}

fn resource_not_found() -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "resource_not_found",
        "Follow request not found",
    )
}

fn target_not_visible() -> Problem {
    Problem::new(
        StatusCode::NOT_FOUND,
        "target_not_visible",
        "User is not available",
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
    use crate::{
        authorization::Role,
        posts::{PostContentRequest, create_reply, get_post, list_replies},
    };
    use axum::http::HeaderMap;
    use miz_api::domain::{PostId, SessionId};

    fn principal(user_id: UserId) -> Principal {
        Principal {
            user_id,
            session_id: SessionId::from_bytes(user_id.to_bytes()),
            role: Role::User,
        }
    }

    #[tokio::test]
    async fn reply_follow_and_timeline_rules_hold() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            smtp_addr: "127.0.0.1:1".to_owned(),
            cursor_signing_key: vec![9; 32],
        };
        let actor = UserId::new().unwrap();
        let public_user = UserId::new().unwrap();
        let private_user = UserId::new().unwrap();
        let outsider = UserId::new().unwrap();
        for (id, name, privacy) in [
            (actor, "Actor", "public"),
            (public_user, "Public", "public"),
            (private_user, "Private", "private"),
            (outsider, "Outsider", "public"),
        ] {
            sqlx::query("INSERT INTO users (id, display_name, privacy) VALUES ($1, $2, $3)")
                .bind(id.to_bytes().to_vec())
                .bind(name)
                .bind(privacy)
                .execute(&pool)
                .await
                .unwrap();
        }
        let actor_principal = principal(actor);
        let public_principal = principal(public_user);
        let private_principal = principal(private_user);
        let outsider_principal = principal(outsider);

        let self_follow = follow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(actor),
        )
        .await
        .unwrap_err();
        assert_eq!(
            self_follow.into_response().status(),
            StatusCode::BAD_REQUEST
        );

        let Json(public_follow) = follow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(public_user),
        )
        .await
        .unwrap();
        assert_eq!(public_follow.status, "accepted");
        let Json(repeated) = follow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(public_user),
        )
        .await
        .unwrap();
        assert_eq!(repeated.id, public_follow.id);

        let Json(private_follow) = follow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(private_user),
        )
        .await
        .unwrap();
        assert_eq!(private_follow.status, "pending");
        let Json(requests) =
            list_follow_requests(State(state.clone()), Extension(private_principal))
                .await
                .unwrap();
        assert_eq!(requests.total, 1);
        let hidden = accept_follow_request(
            State(state.clone()),
            Extension(outsider_principal),
            Path(private_follow.id),
        )
        .await
        .unwrap_err();
        assert_eq!(hidden.into_response().status(), StatusCode::NOT_FOUND);
        let Json(accepted) = accept_follow_request(
            State(state.clone()),
            Extension(private_principal),
            Path(private_follow.id),
        )
        .await
        .unwrap();
        assert_eq!(accepted.status, "accepted");
        let repeated_accept = accept_follow_request(
            State(state.clone()),
            Extension(private_principal),
            Path(private_follow.id),
        )
        .await
        .unwrap_err();
        assert_eq!(
            repeated_accept.into_response().status(),
            StatusCode::CONFLICT
        );

        let actor_post = PostId::new().unwrap();
        let private_post = PostId::new().unwrap();
        let public_post = PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility, created_at) VALUES \
             ($1, $2, 'actor post', 'public', now() - INTERVAL '3 seconds'), \
             ($3, $4, 'private post', 'followers', now() - INTERVAL '2 seconds'), \
             ($5, $6, 'public post', 'public', now() - INTERVAL '1 second')",
        )
        .bind(actor_post.to_bytes().to_vec())
        .bind(actor.to_bytes().to_vec())
        .bind(private_post.to_bytes().to_vec())
        .bind(private_user.to_bytes().to_vec())
        .bind(public_post.to_bytes().to_vec())
        .bind(public_user.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();

        let Json(first_page) = home_timeline(
            State(state.clone()),
            Extension(actor_principal),
            Query(PageQuery {
                limit: Some(1),
                cursor: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(first_page.items[0].id, public_post);
        let Json(second_page) = home_timeline(
            State(state.clone()),
            Extension(actor_principal),
            Query(PageQuery {
                limit: Some(1),
                cursor: first_page.next_cursor,
            }),
        )
        .await
        .unwrap();
        assert_eq!(second_page.items[0].id, private_post);

        sqlx::query("UPDATE users SET privacy = 'private' WHERE id = $1")
            .bind(public_user.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let hidden_after_privacy_change = get_post(
            State(state.clone()),
            Extension(outsider_principal),
            Path(public_post),
        )
        .await
        .unwrap_err();
        assert_eq!(
            hidden_after_privacy_change.into_response().status(),
            StatusCode::NOT_FOUND
        );

        let mut reply_headers = HeaderMap::new();
        reply_headers.insert("idempotency-key", "social-reply-test".parse().unwrap());
        let (_, Json(reply)) = create_reply(
            State(state.clone()),
            Extension(actor_principal),
            Path(private_post),
            reply_headers,
            Json(PostContentRequest {
                content: "reply".to_owned(),
            }),
        )
        .await
        .unwrap();
        let invisible = get_post(
            State(state.clone()),
            Extension(outsider_principal),
            Path(reply.id),
        )
        .await
        .unwrap_err();
        assert_eq!(invisible.into_response().status(), StatusCode::NOT_FOUND);
        sqlx::query(
            "INSERT INTO follow_relationships (id, follower_id, followee_id, status) VALUES ($1, $2, $3, 'accepted')",
        )
        .bind(FollowRelationshipId::new().unwrap().to_bytes().to_vec())
        .bind(outsider.to_bytes().to_vec())
        .bind(private_user.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let Json(visible_reply) = get_post(
            State(state.clone()),
            Extension(outsider_principal),
            Path(reply.id),
        )
        .await
        .unwrap();
        assert_eq!(visible_reply.id, reply.id);
        let Json(replies) = list_replies(
            State(state.clone()),
            Extension(actor_principal),
            Path(private_post),
            Query(PageQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(replies.items.len(), 1);
        let mut nested_headers = HeaderMap::new();
        nested_headers.insert("idempotency-key", "nested-reply-test".parse().unwrap());
        let nested = create_reply(
            State(state.clone()),
            Extension(actor_principal),
            Path(reply.id),
            nested_headers,
            Json(PostContentRequest {
                content: "nested".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(nested.into_response().status(), StatusCode::NOT_FOUND);

        unfollow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(public_user),
        )
        .await
        .unwrap();
        unfollow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(public_user),
        )
        .await
        .unwrap();
        let Json(public_followers) = list_followers(
            State(state.clone()),
            Extension(public_principal),
            Path(public_user),
        )
        .await
        .unwrap();
        assert_eq!(public_followers.total, 0);

        unfollow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(private_user),
        )
        .await
        .unwrap();
        let Json(after_unfollow) = home_timeline(
            State(state),
            Extension(actor_principal),
            Query(PageQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(after_unfollow.items.len(), 1);
        assert_eq!(after_unfollow.items[0].id, actor_post);
    }
}
