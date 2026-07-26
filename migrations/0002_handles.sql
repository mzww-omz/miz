CREATE TABLE handles (
    user_id BYTEA NOT NULL CHECK (octet_length(user_id) = 16),
    value VARCHAR(24) NOT NULL CHECK (value ~ '^[0-9A-Za-z][0-9A-Za-z_]{2,23}$'),
    normalized VARCHAR(24) PRIMARY KEY,
    is_current BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at TIMESTAMPTZ,
    CHECK (normalized = lower(value)),
    CHECK (normalized NOT IN ('admin', 'm1z', 'support')),
    CHECK ((is_current AND retired_at IS NULL) OR (NOT is_current AND retired_at IS NOT NULL))
);

CREATE UNIQUE INDEX handles_current_user_idx ON handles (user_id) WHERE is_current;

COMMENT ON TABLE handles IS 'Permanent handle claims; retired rows remain resolvable to the same user and must not be deleted.';
