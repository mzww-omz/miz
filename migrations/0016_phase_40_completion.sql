-- Social accounts never carry operator privileges.
UPDATE users SET role = 'user' WHERE role <> 'user';
ALTER TABLE users DROP CONSTRAINT users_role_check;
ALTER TABLE users ADD CONSTRAINT users_role_check CHECK (role = 'user');

ALTER TABLE operator_accounts DROP CONSTRAINT operator_accounts_status_check;
ALTER TABLE operator_accounts ADD CONSTRAINT operator_accounts_status_check
  CHECK (status IN ('pending', 'active', 'suspended'));

-- Canonical relationship policy used by every current social surface.
CREATE FUNCTION miz_relationship_allowed(viewer BYTEA, target BYTEA) RETURNS BOOLEAN
LANGUAGE sql STABLE PARALLEL SAFE AS $$
  SELECT viewer = target OR EXISTS (
    SELECT 1 FROM users v, users t
    WHERE v.id = viewer AND t.id = target
      AND v.status = 'active' AND t.status = 'active'
      AND NOT EXISTS (
        SELECT 1 FROM user_blocks b
        WHERE (b.blocker_id = viewer AND b.blocked_id = target)
           OR (b.blocker_id = target AND b.blocked_id = viewer)
      )
  )
$$;

CREATE FUNCTION miz_profile_visible(viewer BYTEA, target BYTEA) RETURNS BOOLEAN
LANGUAGE sql STABLE PARALLEL SAFE AS $$
  SELECT miz_relationship_allowed(viewer, target) AND EXISTS (
    SELECT 1 FROM users target_user
    WHERE target_user.id = target AND target_user.status = 'active'
      AND (viewer = target OR target_user.privacy = 'public' OR EXISTS (
        SELECT 1 FROM follow_relationships relationship
        WHERE relationship.follower_id = viewer
          AND relationship.followee_id = target
          AND relationship.status = 'accepted'
      ))
  )
$$;

CREATE FUNCTION miz_post_visible(viewer BYTEA, target_post BYTEA) RETURNS BOOLEAN
LANGUAGE sql STABLE PARALLEL SAFE AS $$
  WITH RECURSIVE post_chain AS (
    SELECT id, author_id, reply_to_post_id, state, effective_visibility FROM posts WHERE id = target_post
    UNION ALL
    SELECT parent.id, parent.author_id, parent.reply_to_post_id, parent.state, parent.effective_visibility
    FROM posts parent JOIN post_chain child ON parent.id = child.reply_to_post_id
  )
  SELECT EXISTS (SELECT 1 FROM post_chain WHERE id = target_post)
    AND NOT EXISTS (
      SELECT 1 FROM post_chain post
      WHERE post.state NOT IN ('published', 'tombstone')
         OR NOT (
           (post.state = 'tombstone' AND post.author_id = decode(repeat('00', 16), 'hex'))
           OR miz_profile_visible(viewer, post.author_id)
         )
         OR (post.effective_visibility = 'followers'
             AND viewer <> post.author_id
             AND NOT EXISTS (
               SELECT 1 FROM follow_relationships relationship
               WHERE relationship.follower_id = viewer
                 AND relationship.followee_id = post.author_id
                 AND relationship.status = 'accepted'
             ))
    )
$$;

CREATE TABLE operator_mfa_enrollment_challenges (
  token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash) = 32),
  operator_id BYTEA NOT NULL UNIQUE REFERENCES operator_accounts(id) ON DELETE CASCADE,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (expires_at > created_at)
);

CREATE TABLE user_restrictions (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  user_id BYTEA NOT NULL REFERENCES users(id),
  action_id BYTEA NOT NULL UNIQUE REFERENCES moderation_actions(id),
  kind TEXT NOT NULL CHECK (kind IN ('featureRestriction', 'temporarySuspension', 'permanentSuspension')),
  feature TEXT,
  reason TEXT NOT NULL CHECK (btrim(reason) <> '' AND octet_length(reason) <= 8192),
  expires_at TIMESTAMPTZ,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK ((kind = 'featureRestriction') = (feature IS NOT NULL)),
  CHECK ((kind = 'permanentSuspension') = (expires_at IS NULL)),
  CHECK (expires_at IS NULL OR expires_at > created_at)
);

CREATE INDEX user_restrictions_active_user_idx
  ON user_restrictions (user_id, kind, expires_at)
  WHERE revoked_at IS NULL;

CREATE TABLE moderation_appeals (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  action_id BYTEA NOT NULL UNIQUE REFERENCES moderation_actions(id),
  appellant_user_id BYTEA NOT NULL REFERENCES users(id),
  explanation TEXT NOT NULL CHECK (btrim(explanation) <> '' AND octet_length(explanation) <= 8192),
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'upheld', 'overturned')),
  reviewer_operator_id BYTEA REFERENCES operator_accounts(id),
  review_reason TEXT,
  version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  reviewed_at TIMESTAMPTZ,
  CHECK ((state = 'pending') = (reviewer_operator_id IS NULL AND reviewed_at IS NULL))
);

CREATE INDEX moderation_appeals_pending_idx
  ON moderation_appeals (created_at, id) WHERE state = 'pending';

CREATE OR REPLACE FUNCTION reject_audit_log_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' AND OLD.retain_until <= now() THEN
    RETURN OLD;
  END IF;
  RAISE EXCEPTION 'audit_log_entries are append-only during retention';
END;
$$;
