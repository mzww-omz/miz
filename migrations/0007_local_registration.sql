CREATE TABLE registration_attempts (
    id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
    email_normalized TEXT NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'verified', 'profiled', 'completed')),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    CHECK (expires_at > created_at)
);

CREATE TABLE registration_profiles (
    registration_id BYTEA PRIMARY KEY REFERENCES registration_attempts(id) ON DELETE CASCADE,
    handle VARCHAR(24) NOT NULL,
    display_name VARCHAR(50) NOT NULL,
    birth_date DATE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE auth_challenges
    ADD COLUMN registration_id BYTEA UNIQUE REFERENCES registration_attempts(id) ON DELETE CASCADE;

CREATE INDEX registration_attempts_expiry_idx
    ON registration_attempts (expires_at) WHERE status <> 'completed';
