DROP TABLE registration_profiles;
ALTER TABLE auth_challenges DROP COLUMN registration_id;
DROP TABLE registration_attempts;

CREATE TABLE password_credentials (
    user_id BYTEA PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
