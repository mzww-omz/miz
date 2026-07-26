use miz_api::domain::{SessionId, UserId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Principal {
    pub user_id: UserId,
    pub session_id: SessionId,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Mutate,
}

pub fn authorize(principal: Principal, action: Action, owner_id: UserId, _: bool) -> bool {
    match action {
        Action::Mutate => principal.user_id == owner_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_principals_can_only_mutate_their_own_resources() {
        let principal = Principal {
            user_id: UserId::from_bytes([1; 16]),
            session_id: SessionId::from_bytes([1; 16]),
            role: Role::User,
        };
        assert!(authorize(
            principal,
            Action::Mutate,
            principal.user_id,
            false
        ));
        assert!(!authorize(
            principal,
            Action::Mutate,
            UserId::from_bytes([2; 16]),
            false
        ));
    }

    #[tokio::test]
    async fn shared_relationship_policy_covers_privacy_follow_block_and_status() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        miz_api::infrastructure::migrate(&pool).await.unwrap();
        let viewer = UserId::new().unwrap();
        let public = UserId::new().unwrap();
        let private = UserId::new().unwrap();
        for (id, privacy) in [(viewer, "public"), (public, "public"), (private, "private")] {
            sqlx::query(
                "INSERT INTO users (id, display_name, privacy) VALUES ($1, 'Policy user', $2)",
            )
            .bind(id.to_bytes().to_vec())
            .bind(privacy)
            .execute(&pool)
            .await
            .unwrap();
        }
        let visible: bool = sqlx::query_scalar("SELECT miz_profile_visible($1, $2)")
            .bind(viewer.to_bytes().to_vec())
            .bind(public.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(visible);
        let private_visible: bool = sqlx::query_scalar("SELECT miz_profile_visible($1, $2)")
            .bind(viewer.to_bytes().to_vec())
            .bind(private.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!private_visible);
        sqlx::query(
            "INSERT INTO follow_relationships (id, follower_id, followee_id, status) VALUES ($1, $2, $3, 'accepted')",
        )
        .bind(miz_api::domain::FollowRelationshipId::new().unwrap().to_bytes().to_vec())
        .bind(viewer.to_bytes().to_vec())
        .bind(private.to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap();
        let followed: bool = sqlx::query_scalar("SELECT miz_profile_visible($1, $2)")
            .bind(viewer.to_bytes().to_vec())
            .bind(private.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(followed);
        sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
            .bind(private.to_bytes().to_vec())
            .bind(viewer.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let blocked: bool = sqlx::query_scalar("SELECT miz_profile_visible($1, $2)")
            .bind(viewer.to_bytes().to_vec())
            .bind(private.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!blocked);
        sqlx::query("DELETE FROM user_blocks WHERE blocker_id = $1 AND blocked_id = $2")
            .bind(private.to_bytes().to_vec())
            .bind(viewer.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
            .bind(private.to_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let suspended: bool = sqlx::query_scalar("SELECT miz_profile_visible($1, $2)")
            .bind(viewer.to_bytes().to_vec())
            .bind(private.to_bytes().to_vec())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!suspended);
    }
}
