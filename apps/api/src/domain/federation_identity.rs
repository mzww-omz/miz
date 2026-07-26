use super::{Handle, HandleError, UserId};
use std::{fmt, str::FromStr};

pub const DOMAIN: &str = "m1z.jp";
pub const ORIGIN: &str = "https://m1z.jp";

pub fn local_display(handle: &Handle) -> String {
    format!("@{handle}")
}

pub fn profile_url(handle: &Handle) -> String {
    format!("{ORIGIN}/@{handle}")
}

pub fn fediverse_address(handle: &Handle) -> String {
    format!("@{handle}@{DOMAIN}")
}

pub fn actor_id(user_id: UserId) -> String {
    format!("{ORIGIN}/ap/actors/{user_id}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebFingerAccount(pub Handle);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebFingerError {
    InvalidSubject,
    InvalidHandle(HandleError),
}

impl FromStr for WebFingerAccount {
    type Err = WebFingerError;

    fn from_str(subject: &str) -> Result<Self, Self::Err> {
        let account = subject
            .strip_prefix("acct:")
            .and_then(|value| value.strip_suffix(&format!("@{DOMAIN}")))
            .ok_or(WebFingerError::InvalidSubject)?;
        account
            .parse()
            .map(Self)
            .map_err(WebFingerError::InvalidHandle)
    }
}

impl fmt::Display for WebFingerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSubject => formatter.write_str("invalid local WebFinger subject"),
            Self::InvalidHandle(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for WebFingerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_local_and_federated_identity() {
        let handle: Handle = "Miz_User".parse().unwrap();
        assert_eq!(local_display(&handle), "@Miz_User");
        assert_eq!(profile_url(&handle), "https://m1z.jp/@Miz_User");
        assert_eq!(fediverse_address(&handle), "@Miz_User@m1z.jp");
    }

    #[test]
    fn actor_id_depends_on_user_id_not_handle() {
        let user_id = UserId::from_bytes([7; 16]);
        let before: Handle = "old_handle".parse().unwrap();
        let after: Handle = "new_handle".parse().unwrap();
        assert_ne!(profile_url(&before), profile_url(&after));
        assert_eq!(actor_id(user_id), actor_id(user_id));
        assert!(actor_id(user_id).starts_with("https://m1z.jp/ap/actors/"));
    }

    #[test]
    fn parses_only_local_webfinger_accounts_case_insensitively_by_handle() {
        let upper: WebFingerAccount = "acct:Miz_User@m1z.jp".parse().unwrap();
        let lower: WebFingerAccount = "acct:miz_user@m1z.jp".parse().unwrap();
        assert_eq!(upper.0.normalized(), lower.0.normalized());
        assert!(
            "acct:miz_user@example.com"
                .parse::<WebFingerAccount>()
                .is_err()
        );
    }
}
