use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{
    fmt,
    str::FromStr,
    time::{Duration, SystemTime},
};

const RESERVED: &[&str] = &["admin", "m1z", "support"];
const CHANGE_INTERVAL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Handle(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleError {
    Invalid,
    Reserved,
}

impl Handle {
    pub fn normalized(&self) -> String {
        self.0.to_ascii_lowercase()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Display for HandleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => {
                formatter.write_str("handle must match ^[0-9A-Za-z][0-9A-Za-z_]{2,23}$")
            }
            Self::Reserved => formatter.write_str("handle is reserved"),
        }
    }
}

impl std::error::Error for HandleError {}

impl FromStr for Handle {
    type Err = HandleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let valid = (3..=24).contains(&value.len())
            && value.is_ascii()
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if !valid {
            return Err(HandleError::Invalid);
        }
        if RESERVED.contains(&value.to_ascii_lowercase().as_str()) {
            return Err(HandleError::Reserved);
        }
        Ok(Self(value.to_owned()))
    }
}

impl Serialize for Handle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Handle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

pub fn can_change_handle(last_changed_at: Option<SystemTime>, now: SystemTime) -> bool {
    last_changed_at.is_none_or(|last| {
        now.duration_since(last)
            .is_ok_and(|elapsed| elapsed >= CHANGE_INTERVAL)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_case_and_normalizes_for_lookup() {
        let handle: Handle = "Miz_User".parse().unwrap();
        assert_eq!(handle.as_str(), "Miz_User");
        assert_eq!(handle.normalized(), "miz_user");
        assert_eq!(
            handle.normalized(),
            "miz_USER".parse::<Handle>().unwrap().normalized()
        );
    }

    #[test]
    fn rejects_invalid_and_reserved_handles() {
        for value in [
            "ab",
            "_miz",
            "miz-user",
            "ｍｉｚ",
            "admin",
            "Support",
            "M1Z",
        ] {
            assert!(value.parse::<Handle>().is_err(), "accepted {value}");
        }
    }

    #[test]
    fn api_representation_preserves_case_and_validates() {
        let handle: Handle = "Miz_User".parse().unwrap();
        let json = serde_json::to_string(&handle).unwrap();
        assert_eq!(json, "\"Miz_User\"");
        assert_eq!(serde_json::from_str::<Handle>(&json).unwrap(), handle);
        assert!(serde_json::from_str::<Handle>("\"_invalid\"").is_err());
    }

    #[test]
    fn limits_changes_to_every_thirty_days() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100 * 24 * 60 * 60);
        assert!(can_change_handle(None, now));
        assert!(!can_change_handle(
            Some(now - CHANGE_INTERVAL + Duration::from_secs(1)),
            now
        ));
        assert!(can_change_handle(Some(now - CHANGE_INTERVAL), now));
        assert!(!can_change_handle(Some(now + Duration::from_secs(1)), now));
    }
}
