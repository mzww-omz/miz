ALTER TABLE users
    ADD COLUMN role VARCHAR(16) NOT NULL DEFAULT 'user'
    CHECK (role IN ('user', 'moderator', 'administrator'));

CREATE TABLE auth_identities (
    user_id BYTEA NOT NULL REFERENCES users(id),
    provider VARCHAR(32) NOT NULL CHECK (provider IN ('google', 'magic_link')),
    provider_subject TEXT NOT NULL,
    provider_email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, provider_subject),
    UNIQUE (user_id, provider)
);

CREATE TABLE sessions (
    id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
    user_id BYTEA NOT NULL REFERENCES users(id),
    token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    csrf_token_hash BYTEA NOT NULL CHECK (octet_length(csrf_token_hash) = 32),
    device_name TEXT NOT NULL DEFAULT '',
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    idle_expires_at TIMESTAMPTZ NOT NULL,
    absolute_expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (idle_expires_at <= absolute_expires_at),
    CHECK (absolute_expires_at <= created_at + INTERVAL '30 days')
);

CREATE INDEX sessions_user_idx ON sessions (user_id, last_seen_at DESC);
CREATE INDEX sessions_expiry_idx ON sessions (idle_expires_at, absolute_expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE auth_challenges (
    id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
    kind VARCHAR(32) NOT NULL CHECK (kind IN ('google_oauth', 'magic_link')),
    state_hash BYTEA CHECK (state_hash IS NULL OR octet_length(state_hash) = 32),
    nonce_hash BYTEA CHECK (nonce_hash IS NULL OR octet_length(nonce_hash) = 32),
    pkce_verifier_hash BYTEA CHECK (pkce_verifier_hash IS NULL OR octet_length(pkce_verifier_hash) = 32),
    token_hash BYTEA CHECK (token_hash IS NULL OR octet_length(token_hash) = 32),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at)
);

CREATE TABLE rate_limit_windows (
    bucket TEXT NOT NULL,
    bucket_key TEXT NOT NULL,
    window_started_at TIMESTAMPTZ NOT NULL,
    request_count BIGINT NOT NULL CHECK (request_count > 0),
    PRIMARY KEY (bucket, bucket_key, window_started_at)
);

CREATE TABLE security_audit_log (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_user_id BYTEA REFERENCES users(id),
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id BYTEA CHECK (resource_id IS NULL OR octet_length(resource_id) = 16),
    request_id TEXT,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX security_audit_actor_idx ON security_audit_log (actor_user_id, created_at DESC);
