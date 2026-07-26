use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use miz_api::{
    domain::OperatorId,
    infrastructure,
    operator_security::{
        self, base32_secret, encrypt_totp_secret, generate_totp_secret, recovery_code_hash,
    },
};
use sha2::{Digest, Sha256};
use std::io::{self, Read};

#[tokio::main]
async fn main() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let username = normalize_username(
        &std::env::var("OPERATOR_USERNAME").expect("OPERATOR_USERNAME must be set"),
    );
    let encryption_secret = std::env::var("OPERATOR_MFA_ENCRYPTION_KEY")
        .expect("OPERATOR_MFA_ENCRYPTION_KEY must be set");
    assert!(
        encryption_secret.len() >= 32,
        "OPERATOR_MFA_ENCRYPTION_KEY must contain at least 32 bytes"
    );
    let encryption_key: [u8; 32] = Sha256::digest(encryption_secret.as_bytes()).into();
    let mut password = String::new();
    io::stdin()
        .read_to_string(&mut password)
        .expect("operator password must be readable from stdin");
    let password = password.trim_end_matches(['\r', '\n']).to_owned();
    let password_hash = operator_security::hash_password(password).expect("password must be valid");

    let pool = infrastructure::database(&database_url)
        .await
        .expect("database must be reachable");
    infrastructure::migrate(&pool)
        .await
        .expect("database migrations must succeed");
    let mut transaction = pool.begin().await.expect("transaction must start");
    let operator_count: i64 = sqlx::query_scalar("SELECT count(*) FROM operator_accounts")
        .fetch_one(&mut *transaction)
        .await
        .expect("operator count must be readable");
    assert_eq!(
        operator_count, 0,
        "bootstrap is only allowed before the first operator exists"
    );

    let operator_id = OperatorId::new().expect("operator ID generation must succeed");
    let secret = generate_totp_secret().expect("TOTP secret generation must succeed");
    let (encrypted_secret, nonce) =
        encrypt_totp_secret(&encryption_key, &secret).expect("TOTP encryption must succeed");
    sqlx::query(
        "INSERT INTO operator_accounts (id, username, normalized_username) VALUES ($1, $2, $2)",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(&username)
    .execute(&mut *transaction)
    .await
    .expect("operator account must be created");
    sqlx::query("INSERT INTO operator_credentials (operator_id, password_hash) VALUES ($1, $2)")
        .bind(operator_id.to_bytes().to_vec())
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .expect("operator credential must be created");
    sqlx::query(
        "INSERT INTO operator_mfa_factors (operator_id, encrypted_totp_secret, encryption_nonce) VALUES ($1, $2, $3)",
    )
    .bind(operator_id.to_bytes().to_vec())
    .bind(encrypted_secret)
    .bind(nonce.to_vec())
    .execute(&mut *transaction)
    .await
    .expect("operator MFA factor must be created");
    sqlx::query(
        "INSERT INTO operator_role_assignments (operator_id, role) VALUES ($1, 'administrator')",
    )
    .bind(operator_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .expect("administrator role must be assigned");

    let mut recovery_codes = Vec::with_capacity(10);
    for _ in 0..10 {
        let mut bytes = [0_u8; 18];
        getrandom::fill(&mut bytes).expect("recovery code generation must succeed");
        let code = URL_SAFE_NO_PAD.encode(bytes);
        sqlx::query("INSERT INTO operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)")
            .bind(operator_id.to_bytes().to_vec())
            .bind(recovery_code_hash(&code).to_vec())
            .execute(&mut *transaction)
            .await
            .expect("recovery code must be stored");
        recovery_codes.push(code);
    }
    sqlx::query(
        "INSERT INTO audit_log_entries (actor_operator_id, event_type, target_type, target_id, reason) \
         VALUES ($1, 'operatorBootstrap', 'operator', $1, 'Initial administrator bootstrap')",
    )
    .bind(operator_id.to_bytes().to_vec())
    .execute(&mut *transaction)
    .await
    .expect("bootstrap audit record must be stored");
    transaction.commit().await.expect("bootstrap must commit");

    println!(
        "otpauth://totp/MIZ:{username}?secret={}&issuer=MIZ&algorithm=SHA1&digits=6&period=30",
        base32_secret(&secret)
    );
    println!("Recovery codes (store once, then remove this output):");
    for code in recovery_codes {
        println!("{code}");
    }
}

fn normalize_username(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    assert!(
        (3..=64).contains(&normalized.len())
            && normalized
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')),
        "OPERATOR_USERNAME is invalid"
    );
    normalized
}
