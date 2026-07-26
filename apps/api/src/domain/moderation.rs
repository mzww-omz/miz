#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorRole {
    Support,
    Moderator,
    SeniorModerator,
    Administrator,
    Auditor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorPermission {
    ReadBasicAccount,
    ReviewReportEvidence,
    RemoveContent,
    ApplyTemporaryRestriction,
    TemporarilySuspendAccount,
    ReviewAppeal,
    PermanentlySuspendAccount,
    ManageOperatorRoles,
    ReadAuditLog,
}

pub fn operator_authorized(role: OperatorRole, permission: OperatorPermission) -> bool {
    use OperatorPermission as Permission;
    use OperatorRole as Role;

    match role {
        Role::Support => permission == Permission::ReadBasicAccount,
        Role::Moderator => matches!(
            permission,
            Permission::ReviewReportEvidence
                | Permission::RemoveContent
                | Permission::ApplyTemporaryRestriction
        ),
        Role::SeniorModerator => matches!(
            permission,
            Permission::ReviewReportEvidence
                | Permission::RemoveContent
                | Permission::ApplyTemporaryRestriction
                | Permission::TemporarilySuspendAccount
                | Permission::ReviewAppeal
        ),
        Role::Administrator => matches!(
            permission,
            Permission::ReviewReportEvidence
                | Permission::RemoveContent
                | Permission::ApplyTemporaryRestriction
                | Permission::TemporarilySuspendAccount
                | Permission::ReviewAppeal
                | Permission::PermanentlySuspendAccount
                | Permission::ManageOperatorRoles
        ),
        Role::Auditor => permission == Permission::ReadAuditLog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_roles_are_deny_by_default_and_separate_audit_access() {
        assert!(operator_authorized(
            OperatorRole::Support,
            OperatorPermission::ReadBasicAccount
        ));
        assert!(!operator_authorized(
            OperatorRole::Support,
            OperatorPermission::ReviewReportEvidence
        ));
        assert!(operator_authorized(
            OperatorRole::Moderator,
            OperatorPermission::RemoveContent
        ));
        assert!(!operator_authorized(
            OperatorRole::Moderator,
            OperatorPermission::PermanentlySuspendAccount
        ));
        assert!(operator_authorized(
            OperatorRole::SeniorModerator,
            OperatorPermission::ReviewAppeal
        ));
        assert!(operator_authorized(
            OperatorRole::Administrator,
            OperatorPermission::ManageOperatorRoles
        ));
        assert!(!operator_authorized(
            OperatorRole::Administrator,
            OperatorPermission::ReadAuditLog
        ));
        assert!(operator_authorized(
            OperatorRole::Auditor,
            OperatorPermission::ReadAuditLog
        ));
        assert!(!operator_authorized(
            OperatorRole::Auditor,
            OperatorPermission::RemoveContent
        ));
    }
}
