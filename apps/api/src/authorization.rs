use miz_api::domain::{SessionId, UserId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Moderator,
    Administrator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Principal {
    pub user_id: UserId,
    pub session_id: SessionId,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ReadPublic,
    ReadFollowersOnly,
    Mutate,
    Moderate,
    Administer,
}

pub fn authorize(
    principal: Principal,
    action: Action,
    owner_id: UserId,
    follows_owner: bool,
) -> bool {
    match action {
        Action::ReadPublic => true,
        Action::ReadFollowersOnly => {
            principal.user_id == owner_id
                || follows_owner
                || matches!(principal.role, Role::Moderator | Role::Administrator)
        }
        Action::Mutate => principal.user_id == owner_id,
        Action::Moderate => matches!(principal.role, Role::Moderator | Role::Administrator),
        Action::Administer => principal.role == Role::Administrator,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(byte: u8, role: Role) -> Principal {
        Principal {
            user_id: UserId::from_bytes([byte; 16]),
            session_id: SessionId::from_bytes([byte; 16]),
            role,
        }
    }

    #[test]
    fn authorization_is_object_scoped_and_denies_unlisted_privileges() {
        let owner = UserId::from_bytes([1; 16]);
        let stranger = principal(2, Role::User);
        assert!(authorize(stranger, Action::ReadPublic, owner, false));
        assert!(!authorize(
            stranger,
            Action::ReadFollowersOnly,
            owner,
            false
        ));
        assert!(authorize(stranger, Action::ReadFollowersOnly, owner, true));
        assert!(!authorize(stranger, Action::Mutate, owner, true));
        assert!(!authorize(stranger, Action::Moderate, owner, false));
        assert!(!authorize(stranger, Action::Administer, owner, false));
        assert!(authorize(
            principal(3, Role::Moderator),
            Action::Moderate,
            owner,
            false
        ));
        assert!(authorize(
            principal(4, Role::Administrator),
            Action::Administer,
            owner,
            false
        ));
    }
}
