CREATE TABLE content_reports (
  id BYTEA PRIMARY KEY CHECK (octet_length(id) = 16),
  reporter_id BYTEA NOT NULL REFERENCES users(id),
  target_post_id BYTEA NOT NULL REFERENCES posts(id),
  reason VARCHAR(32) NOT NULL CHECK (reason IN (
    'spam', 'harassment', 'hatefulContent', 'violence', 'sexualContent',
    'illegalOrDangerousTrade', 'personalInformation', 'copyright', 'other'
  )),
  explanation TEXT CHECK (explanation IS NULL OR (btrim(explanation) <> '' AND octet_length(explanation) <= 8192)),
  state VARCHAR(16) NOT NULL DEFAULT 'received' CHECK (state IN ('received', 'inReview', 'actioned', 'noAction')),
  version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (reason <> 'other' OR explanation IS NOT NULL)
);

CREATE UNIQUE INDEX content_reports_unresolved_reporter_target_idx
  ON content_reports (reporter_id, target_post_id)
  WHERE state IN ('received', 'inReview');
CREATE INDEX content_reports_reporter_created_idx
  ON content_reports (reporter_id, created_at DESC);
CREATE INDEX content_reports_queue_idx
  ON content_reports (state, created_at, id);

CREATE TABLE content_report_evidence (
  report_id BYTEA PRIMARY KEY REFERENCES content_reports(id) ON DELETE CASCADE,
  target_kind VARCHAR(8) NOT NULL CHECK (target_kind IN ('post', 'reply')),
  target_id BYTEA NOT NULL CHECK (octet_length(target_id) = 16),
  author_id BYTEA NOT NULL CHECK (octet_length(author_id) = 16),
  content TEXT NOT NULL,
  target_version BIGINT NOT NULL CHECK (target_version > 0),
  target_created_at TIMESTAMPTZ NOT NULL,
  attachment_references JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(attachment_references) = 'array'),
  retain_until TIMESTAMPTZ,
  captured_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX content_report_evidence_retention_idx
  ON content_report_evidence (retain_until, report_id)
  WHERE retain_until IS NOT NULL;
