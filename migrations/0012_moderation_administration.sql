CREATE TABLE operator_accounts (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  username VARCHAR(64) NOT NULL,
  normalized_username VARCHAR(64) NOT NULL UNIQUE,
  status VARCHAR(16) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE operator_credentials (
  operator_id BYTEA PRIMARY KEY REFERENCES operator_accounts(id) ON DELETE CASCADE,
  password_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE operator_mfa_factors (
  operator_id BYTEA PRIMARY KEY REFERENCES operator_accounts(id) ON DELETE CASCADE,
  encrypted_totp_secret BYTEA NOT NULL,
  encryption_nonce BYTEA NOT NULL,
  enabled_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_used_step BIGINT
);

CREATE TABLE operator_recovery_codes (
  operator_id BYTEA NOT NULL REFERENCES operator_accounts(id) ON DELETE CASCADE,
  code_hash BYTEA NOT NULL CHECK (octet_length(code_hash) = 32),
  used_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (operator_id, code_hash)
);

CREATE TABLE operator_sessions (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  operator_id BYTEA NOT NULL REFERENCES operator_accounts(id) ON DELETE CASCADE,
  token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
  mfa_verified_at TIMESTAMPTZ NOT NULL,
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  idle_expires_at TIMESTAMPTZ NOT NULL,
  absolute_expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (idle_expires_at <= absolute_expires_at)
);

CREATE INDEX operator_sessions_active_idx
  ON operator_sessions (operator_id, idle_expires_at)
  WHERE revoked_at IS NULL;

CREATE TABLE operator_role_assignments (
  operator_id BYTEA NOT NULL REFERENCES operator_accounts(id) ON DELETE CASCADE,
  role VARCHAR(24) NOT NULL CHECK (role IN ('support', 'moderator', 'seniorModerator', 'administrator', 'auditor')),
  granted_by BYTEA REFERENCES operator_accounts(id),
  granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (operator_id, role)
);

CREATE TABLE moderation_actions (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  actor_operator_id BYTEA NOT NULL REFERENCES operator_accounts(id),
  report_id BYTEA REFERENCES content_reports(id),
  action_type VARCHAR(32) NOT NULL CHECK (action_type IN ('removeContent', 'temporaryRestriction', 'temporarySuspension', 'permanentSuspension', 'restoreContent', 'roleChange')),
  target_type VARCHAR(16) NOT NULL CHECK (target_type IN ('user', 'post', 'operator')),
  target_id BYTEA NOT NULL CHECK (octet_length(target_id) = 16),
  reason TEXT NOT NULL CHECK (btrim(reason) <> '' AND octet_length(reason) <= 8192),
  before_state JSONB NOT NULL,
  after_state JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX moderation_actions_report_idx ON moderation_actions (report_id, created_at DESC);
CREATE INDEX moderation_actions_target_idx ON moderation_actions (target_type, target_id, created_at DESC);

CREATE TABLE audit_log_entries (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_operator_id BYTEA REFERENCES operator_accounts(id),
  event_type VARCHAR(64) NOT NULL,
  target_type VARCHAR(32),
  target_id BYTEA,
  reason TEXT,
  before_state JSONB,
  after_state JSONB,
  request_id BYTEA CHECK (request_id IS NULL OR octet_length(request_id) = 16),
  report_id BYTEA REFERENCES content_reports(id),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  retain_until TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '1 year'),
  CHECK ((target_type IS NULL) = (target_id IS NULL))
);

CREATE INDEX audit_log_entries_created_idx ON audit_log_entries (created_at DESC, id DESC);
CREATE INDEX audit_log_entries_target_idx ON audit_log_entries (target_type, target_id, created_at DESC);

CREATE FUNCTION reject_audit_log_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'audit_log_entries are append-only';
END;
$$;

CREATE TRIGGER audit_log_entries_append_only
BEFORE UPDATE OR DELETE ON audit_log_entries
FOR EACH ROW EXECUTE FUNCTION reject_audit_log_mutation();
