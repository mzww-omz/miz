use crate::{api::Problem, authorization::Principal, security::SecurityState};
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use miz_api::domain::UserId;

pub async fn block_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(target_id): Path<UserId>,
) -> Result<Response, Problem> {
    if principal.user_id == target_id {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "cannot_block_self",
            "A user cannot block themselves",
        ));
    }
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    lock_user_pair(&mut transaction, principal.user_id, target_id).await?;
    ensure_target_visible(&mut transaction, principal.user_id, target_id, true).await?;
    sqlx::query(
        "INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(target_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    sqlx::query(
        "UPDATE follow_relationships SET status = 'removed', updated_at = now() \
         WHERE status IN ('pending', 'accepted') AND ((follower_id = $1 AND followee_id = $2) OR (follower_id = $2 AND followee_id = $1))",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(target_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn unblock_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(target_id): Path<UserId>,
) -> Result<Response, Problem> {
    sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
        .bind(principal.user_id.to_bytes().to_vec())
        .bind(target_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn mute_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(target_id): Path<UserId>,
) -> Result<Response, Problem> {
    if principal.user_id == target_id {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "cannot_mute_self",
            "A user cannot mute themselves",
        ));
    }
    let mut transaction = state.pool.begin().await.map_err(internal_error)?;
    ensure_target_visible(&mut transaction, principal.user_id, target_id, false).await?;
    sqlx::query(
        "INSERT INTO user_mutes (muter_id, muted_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(principal.user_id.to_bytes().to_vec())
    .bind(target_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;
    transaction.commit().await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub async fn unmute_user(
    State(state): State<SecurityState>,
    Extension(principal): Extension<Principal>,
    Path(target_id): Path<UserId>,
) -> Result<Response, Problem> {
    sqlx::query("DELETE FROM user_mutes WHERE muter_id = $1 AND muted_id = $2")
        .bind(principal.user_id.to_bytes().to_vec())
        .bind(target_id.to_bytes().to_vec())
        .execute(&state.pool)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(crate) async fn lock_user_pair(
    connection: &mut sqlx::PgConnection,
    first: UserId,
    second: UserId,
) -> Result<(), Problem> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended(encode(LEAST($1::bytea, $2::bytea), 'hex') || encode(GREATEST($1::bytea, $2::bytea), 'hex'), 0))",
    )
    .bind(first.to_bytes().to_vec())
    .bind(second.to_bytes().to_vec())
    .execute(connection)
    .await
    .map_err(internal_error)?;
    Ok(())
}

async fn ensure_target_visible(
    connection: &mut sqlx::PgConnection,
    actor_id: UserId,
    user_id: UserId,
    allow_existing_block: bool,
) -> Result<(), Problem> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND status = 'active' \
         AND (miz_relationship_allowed($2, id) OR ($3 AND EXISTS ( \
           SELECT 1 FROM user_blocks WHERE blocker_id = $2 AND blocked_id = id))))",
    )
    .bind(user_id.to_bytes().to_vec())
    .bind(actor_id.to_bytes().to_vec())
    .bind(allow_existing_block)
    .fetch_one(connection)
    .await
    .map_err(internal_error)?;
    if exists {
        Ok(())
    } else {
        Err(Problem::new(
            StatusCode::NOT_FOUND,
            "target_not_visible",
            "Target user not found",
        ))
    }
}

fn internal_error(error: impl std::fmt::Display) -> Problem {
    tracing::error!(error = %error, "relationship operation failed");
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An internal error occurred",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authorization::Role,
        pagination::PageQuery,
        posts::get_post,
        social::{follow_user, home_timeline},
    };
    use axum::{Json, extract::Query};
    use miz_api::domain::{PostId, SessionId};

    fn principal(user_id: UserId) -> Principal {
        Principal {
            user_id,
            session_id: SessionId::from_bytes(user_id.to_bytes()),
            role: Role::User,
        }
    }

    #[tokio::test]
    async fn block_and_mute_apply_to_current_social_surfaces() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![7; 32],
            operator_mfa_key: [7; 32],
        };
        let actor = UserId::new().unwrap();
        let target = UserId::new().unwrap();
        for (id, name) in [(actor, "Block actor"), (target, "Block target")] {
            sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
                .bind(id.to_bytes().to_vec())
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        let target_post = PostId::new().unwrap();
        sqlx::query(
            "INSERT INTO posts (id, author_id, content, effective_visibility) VALUES ($1, $2, 'target post', 'public')",
        )
        .bind(target_post.to_bytes().to_vec())
        .bind(target.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let actor_principal = principal(actor);

        let _ = follow_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(target),
        )
        .await
        .unwrap();
        mute_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(target),
        )
        .await
        .unwrap();
        let Json(muted_timeline) = home_timeline(
            State(state.clone()),
            Extension(actor_principal),
            Query(PageQuery::default()),
        )
        .await
        .unwrap();
        assert!(muted_timeline.items.is_empty());
        let status: String = sqlx::query_scalar(
            "SELECT status FROM follow_relationships WHERE follower_id = $1 AND followee_id = $2",
        )
        .bind(actor.to_bytes().to_vec())
        .bind(target.to_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "accepted");

        unmute_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(target),
        )
        .await
        .unwrap();
        block_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(target),
        )
        .await
        .unwrap();
        block_user(
            State(state.clone()),
            Extension(actor_principal),
            Path(target),
        )
        .await
        .unwrap();
        let status: String = sqlx::query_scalar(
            "SELECT status FROM follow_relationships WHERE follower_id = $1 AND followee_id = $2",
        )
        .bind(actor.to_bytes().to_vec())
        .bind(target.to_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "removed");
        assert!(
            follow_user(
                State(state.clone()),
                Extension(actor_principal),
                Path(target),
            )
            .await
            .is_err()
        );
        assert!(
            get_post(
                State(state.clone()),
                Extension(actor_principal),
                Path(target_post),
            )
            .await
            .is_err()
        );
        unblock_user(State(state), Extension(actor_principal), Path(target))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn concurrent_follow_cannot_survive_block_creation() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let state = SecurityState {
            pool: pool.clone(),
            origin: "https://m1z.jp".to_owned(),
            cursor_signing_key: vec![9; 32],
            operator_mfa_key: [9; 32],
        };
        let actor = UserId::new().unwrap();
        let target = UserId::new().unwrap();
        for id in [actor, target] {
            sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Concurrent user')")
                .bind(id.to_bytes().to_vec())
                .execute(&pool)
                .await
                .unwrap();
        }
        let (blocked, _) = tokio::join!(
            block_user(
                State(state.clone()),
                Extension(principal(actor)),
                Path(target)
            ),
            follow_user(State(state), Extension(principal(actor)), Path(target)),
        );
        blocked.unwrap();
        let active_relationships: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM follow_relationships WHERE follower_id = $1 AND followee_id = $2 AND status IN ('pending', 'accepted')",
        )
        .bind(actor.to_bytes().to_vec())
        .bind(target.to_bytes().to_vec())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(active_relationships, 0);
    }
}
