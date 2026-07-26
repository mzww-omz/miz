CREATE TABLE users (
    id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
    display_name VARCHAR(50) NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 50),
    bio TEXT NOT NULL DEFAULT '',
    privacy VARCHAR(16) NOT NULL DEFAULT 'public' CHECK (privacy IN ('public', 'private')),
    status VARCHAR(16) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'deleted')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE handles
    ADD CONSTRAINT handles_user_fk FOREIGN KEY (user_id) REFERENCES users(id);

CREATE TABLE posts (
    id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
    author_id BYTEA NOT NULL REFERENCES users(id),
    reply_to_post_id BYTEA REFERENCES posts(id),
    content TEXT NOT NULL CHECK (btrim(content) <> '' AND octet_length(content) <= 8192),
    effective_visibility VARCHAR(16) NOT NULL CHECK (effective_visibility IN ('public', 'followers')),
    state VARCHAR(16) NOT NULL DEFAULT 'published' CHECK (state IN ('published', 'deleted', 'tombstone')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ,
    CHECK (reply_to_post_id IS NULL OR reply_to_post_id <> id),
    CHECK ((state = 'published' AND deleted_at IS NULL) OR (state <> 'published' AND deleted_at IS NOT NULL))
);

CREATE INDEX posts_author_created_idx ON posts (author_id, created_at DESC, id DESC);
CREATE INDEX posts_reply_created_idx ON posts (reply_to_post_id, created_at ASC, id ASC)
    WHERE reply_to_post_id IS NOT NULL;

CREATE TABLE post_mentions (
    post_id BYTEA NOT NULL REFERENCES posts(id),
    user_id BYTEA NOT NULL REFERENCES users(id),
    display_text VARCHAR(50) NOT NULL CHECK (char_length(display_text) BETWEEN 1 AND 50),
    PRIMARY KEY (post_id, user_id)
);

CREATE INDEX post_mentions_user_idx ON post_mentions (user_id, post_id);

CREATE TABLE follow_relationships (
    id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
    follower_id BYTEA NOT NULL REFERENCES users(id),
    followee_id BYTEA NOT NULL REFERENCES users(id),
    status VARCHAR(16) NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected', 'cancelled', 'removed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (follower_id <> followee_id),
    UNIQUE (follower_id, followee_id)
);

CREATE INDEX follow_followee_status_idx
    ON follow_relationships (followee_id, status, created_at DESC);
CREATE INDEX follow_follower_status_idx
    ON follow_relationships (follower_id, status, created_at DESC);
