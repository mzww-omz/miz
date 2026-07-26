//! Business rules and domain types.

mod federation_identity;
mod handle;
mod moderation;
mod object_id;
mod post;

pub use federation_identity::{
    DOMAIN, ORIGIN, WebFingerAccount, WebFingerError, actor_id, fediverse_address, local_display,
    profile_url,
};
pub use handle::{Handle, HandleError, can_change_handle};
pub use moderation::{OperatorPermission, OperatorRole, operator_authorized};
pub use object_id::{
    AccountDeletionRequestId, AppealId, FollowRelationshipId, ObjectIdError, OperatorId, PostId,
    ReportId, RequestId, RestrictionId, SessionId, UserId,
};
pub use post::{PostContent, PostContentError};
