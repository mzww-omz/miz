use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ENCODED_LEN: usize = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectIdError {
    InvalidFormat,
    OutOfRange,
}

impl fmt::Display for ObjectIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("object ID must match ^[0-9A-Za-z]{22}$"),
            Self::OutOfRange => formatter.write_str("object ID exceeds 128 bits"),
        }
    }
}

impl std::error::Error for ObjectIdError {}

fn encode(bytes: [u8; 16]) -> String {
    let mut value = u128::from_be_bytes(bytes);
    let mut encoded = [b'0'; ENCODED_LEN];
    for character in encoded.iter_mut().rev() {
        *character = ALPHABET[(value % 62) as usize];
        value /= 62;
    }
    String::from_utf8(encoded.to_vec()).expect("Base62 is valid UTF-8")
}

fn decode(value: &str) -> Result<[u8; 16], ObjectIdError> {
    if value.len() != ENCODED_LEN || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(ObjectIdError::InvalidFormat);
    }

    let decoded = value.bytes().try_fold(0_u128, |result, byte| {
        let digit = ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .expect("validated Base62 character") as u128;
        result
            .checked_mul(62)
            .and_then(|number| number.checked_add(digit))
            .ok_or(ObjectIdError::OutOfRange)
    })?;
    Ok(decoded.to_be_bytes())
}

macro_rules! object_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
        pub struct $name([u8; 16]);

        impl $name {
            pub fn new() -> Result<Self, getrandom::Error> {
                let mut bytes = [0; 16];
                getrandom::fill(&mut bytes)?;
                Ok(Self(bytes))
            }

            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn to_bytes(self) -> [u8; 16] {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&encode(self.0))
            }
        }

        impl FromStr for $name {
            type Err = ObjectIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                decode(value).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(de::Error::custom)
            }
        }
    };
}

object_id!(
    /// Public identifier for a local user.
    ///
    /// ```compile_fail
    /// use miz_api::domain::{PostId, UserId};
    /// let post = PostId::from_bytes([0; 16]);
    /// let user: UserId = post;
    /// ```
    UserId
);
object_id!(PostId);
object_id!(FollowRelationshipId);
object_id!(ReportId);
object_id!(RequestId);
object_id!(SessionId);
object_id!(RegistrationId);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generates_unique_base62_ids() {
        let ids: HashSet<_> = (0..10_000)
            .map(|_| UserId::new().unwrap().to_string())
            .collect();
        assert_eq!(ids.len(), 10_000);
        assert!(
            ids.iter()
                .all(|id| id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        );
    }

    #[test]
    fn converts_all_128_bits_without_loss() {
        for bytes in [[0; 16], [0xff; 16], [0x5a; 16]] {
            let id = PostId::from_bytes(bytes);
            assert_eq!(id.to_string().parse::<PostId>().unwrap(), id);
            assert_eq!(id.to_bytes(), bytes);
        }
    }

    #[test]
    fn rejects_invalid_and_out_of_range_ids() {
        assert_eq!("short".parse::<UserId>(), Err(ObjectIdError::InvalidFormat));
        assert_eq!(
            "000000000000000000000_".parse::<UserId>(),
            Err(ObjectIdError::InvalidFormat)
        );
        assert_eq!(
            "zzzzzzzzzzzzzzzzzzzzzz".parse::<UserId>(),
            Err(ObjectIdError::OutOfRange)
        );
    }

    #[test]
    fn parsing_is_case_sensitive() {
        let upper = "000000000000000000000A".parse::<SessionId>().unwrap();
        let lower = "000000000000000000000a".parse::<SessionId>().unwrap();
        assert_ne!(upper, lower);
    }

    #[test]
    fn api_representation_is_base62() {
        let id = ReportId::from_bytes([0; 16]);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"0000000000000000000000\"");
        assert_eq!(serde_json::from_str::<ReportId>(&json).unwrap(), id);
    }
}
