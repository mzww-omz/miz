CREATE TABLE account_deletion_requests (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  user_id BYTEA NOT NULL REFERENCES users(id),
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending', 'cancelled', 'purging', 'purged', 'restored')),
  requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  restore_until TIMESTAMPTZ NOT NULL,
  claimed_at TIMESTAMPTZ,
  completed_at TIMESTAMPTZ,
  CHECK (restore_until = requested_at + INTERVAL '30 days'),
  CHECK (state <> 'purging' OR (claimed_at IS NOT NULL AND completed_at IS NULL)),
  CHECK ((state IN ('cancelled', 'purged', 'restored')) = (completed_at IS NOT NULL))
);

CREATE UNIQUE INDEX account_deletion_requests_active_user_idx
  ON account_deletion_requests (user_id)
  WHERE state IN ('pending', 'purging');
CREATE INDEX account_deletion_requests_due_idx
  ON account_deletion_requests (restore_until, id)
  WHERE state = 'pending';

CREATE TABLE maintenance_jobs (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('purgeAccount')),
  account_deletion_request_id BYTEA NOT NULL UNIQUE
    REFERENCES account_deletion_requests(id) ON DELETE CASCADE,
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending', 'claimed', 'completed', 'failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  available_at TIMESTAMPTZ NOT NULL,
  claimed_at TIMESTAMPTZ,
  completed_at TIMESTAMPTZ,
  last_error_code TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX maintenance_jobs_due_idx
  ON maintenance_jobs (available_at, id)
  WHERE state IN ('pending', 'failed');

-- Deleted content keeps thread structure but no longer points to the original account.
INSERT INTO users (id, display_name, privacy, status)
VALUES (decode(repeat('00', 16), 'hex'), 'Deleted account', 'private', 'deleted')
ON CONFLICT (id) DO NOTHING;
