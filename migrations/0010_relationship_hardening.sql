CREATE TABLE user_blocks (
  blocker_id BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  blocked_id BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (blocker_id, blocked_id),
  CHECK (blocker_id <> blocked_id)
);

CREATE INDEX user_blocks_blocked_idx ON user_blocks (blocked_id, blocker_id);

CREATE TABLE user_mutes (
  muter_id BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  muted_id BYTEA NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (muter_id, muted_id),
  CHECK (muter_id <> muted_id)
);
