use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use subtle::ConstantTimeEq;

pub const MINIMUM_PASSWORD_BYTES: usize = 12;
pub const MAXIMUM_PASSWORD_BYTES: usize = 128;

pub fn hash_password(password: String) -> Result<String, String> {
    validate_password(&password)?;
    let mut salt = [0_u8; 16];
    getrandom::fill(&mut salt).map_err(|error| error.to_string())?;
    let salt = SaltString::encode_b64(&salt).map_err(|error| error.to_string())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

pub fn dummy_password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password("invalid password".to_owned()).expect("dummy password is valid")
    })
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn validate_password(password: &str) -> Result<(), String> {
    if (MINIMUM_PASSWORD_BYTES..=MAXIMUM_PASSWORD_BYTES).contains(&password.len()) {
        Ok(())
    } else {
        Err(format!(
            "password must contain {MINIMUM_PASSWORD_BYTES} to {MAXIMUM_PASSWORD_BYTES} UTF-8 bytes"
        ))
    }
}

pub fn generate_totp_secret() -> Result<[u8; 20], getrandom::Error> {
    let mut secret = [0_u8; 20];
    getrandom::fill(&mut secret)?;
    Ok(secret)
}

pub fn base32_secret(secret: &[u8]) -> String {
    BASE32_NOPAD.encode(secret)
}

pub fn encrypt_totp_secret(key: &[u8; 32], secret: &[u8]) -> Result<(Vec<u8>, [u8; 12]), String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| error.to_string())?;
    let mut nonce = [0_u8; 12];
    getrandom::fill(&mut nonce).map_err(|error| error.to_string())?;
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), secret)
        .map_err(|_| "TOTP encryption failed".to_owned())?;
    Ok((encrypted, nonce))
}

pub fn decrypt_totp_secret(
    key: &[u8; 32],
    encrypted: &[u8],
    nonce: &[u8],
) -> Result<Vec<u8>, String> {
    if nonce.len() != 12 {
        return Err("invalid TOTP nonce".to_owned());
    }
    Aes256Gcm::new_from_slice(key)
        .map_err(|error| error.to_string())?
        .decrypt(Nonce::from_slice(nonce), encrypted)
        .map_err(|_| "TOTP decryption failed".to_owned())
}

pub fn totp_code(secret: &[u8], unix_seconds: u64) -> String {
    format!("{:06}", totp_value(secret, unix_seconds / 30))
}

pub fn verify_totp(secret: &[u8], code: &str, unix_seconds: u64) -> Option<u64> {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let expected = code.as_bytes();
    let current_step = unix_seconds / 30;
    for step in current_step.saturating_sub(1)..=current_step.saturating_add(1) {
        let candidate = format!("{:06}", totp_value(secret, step));
        if bool::from(candidate.as_bytes().ct_eq(expected)) {
            return Some(step);
        }
    }
    None
}

pub fn recovery_code_hash(code: &str) -> [u8; 32] {
    Sha256::digest(code.as_bytes()).into()
}

pub fn generate_recovery_codes() -> Result<Vec<String>, getrandom::Error> {
    (0..10)
        .map(|_| {
            let mut bytes = [0_u8; 18];
            getrandom::fill(&mut bytes)?;
            Ok(URL_SAFE_NO_PAD.encode(bytes))
        })
        .collect()
}

fn totp_value(secret: &[u8], step: u64) -> u32 {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let value = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    value % 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_matches_rfc_6238_sha1_vector_at_six_digits() {
        let secret = b"12345678901234567890";
        assert_eq!(totp_code(secret, 59), "287082");
        assert_eq!(verify_totp(secret, "287082", 59), Some(1));
        assert_eq!(verify_totp(secret, "287083", 59), None);
    }

    #[test]
    fn encrypted_totp_secrets_reject_wrong_keys() {
        let key = [7_u8; 32];
        let secret = generate_totp_secret().unwrap();
        let (encrypted, nonce) = encrypt_totp_secret(&key, &secret).unwrap();
        assert_eq!(
            decrypt_totp_secret(&key, &encrypted, &nonce).unwrap(),
            secret
        );
        assert!(decrypt_totp_secret(&[8_u8; 32], &encrypted, &nonce).is_err());
    }

    #[test]
    fn password_hashes_are_salted() {
        let first = hash_password("correct horse battery staple".to_owned()).unwrap();
        let second = hash_password("correct horse battery staple".to_owned()).unwrap();
        assert_ne!(first, second);
        assert!(verify_password("correct horse battery staple", &first));
        assert!(!verify_password("wrong password", &first));
    }
}
