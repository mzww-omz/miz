ALTER TABLE posts
    ADD COLUMN edited_at TIMESTAMPTZ;

ALTER TABLE posts
    ALTER COLUMN content DROP NOT NULL,
    DROP CONSTRAINT posts_content_check,
    ADD CONSTRAINT posts_content_state_check CHECK (
        (state = 'published' AND content IS NOT NULL AND btrim(content) <> '' AND octet_length(content) <= 8192)
        OR (state IN ('deleted', 'tombstone') AND content IS NULL)
    );

CREATE TABLE post_edit_history (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    post_id BYTEA NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    editor_id BYTEA NOT NULL REFERENCES users(id),
    previous_content TEXT NOT NULL,
    new_content TEXT NOT NULL,
    edited_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX post_edit_history_post_idx ON post_edit_history (post_id, edited_at DESC);

CREATE TABLE idempotency_keys (
    user_id BYTEA NOT NULL REFERENCES users(id),
    endpoint TEXT NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash) = 32),
    response_status INTEGER,
    response_body JSONB,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, endpoint, idempotency_key),
    CHECK ((response_status IS NULL) = (response_body IS NULL))
);

CREATE INDEX idempotency_keys_expiry_idx ON idempotency_keys (expires_at);
