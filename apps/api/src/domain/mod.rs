//! Business rules and domain types.

mod federation_identity;
mod handle;
mod object_id;
mod post;

pub use federation_identity::{
    DOMAIN, ORIGIN, WebFingerAccount, WebFingerError, actor_id, fediverse_address, local_display,
    profile_url,
};
pub use handle::{Handle, HandleError, can_change_handle};
pub use object_id::{
    FollowRelationshipId, ObjectIdError, PostId, RegistrationId, ReportId, RequestId, SessionId,
    UserId,
};
pub use post::{PostContent, PostContentError};
