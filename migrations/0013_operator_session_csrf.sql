ALTER TABLE operator_sessions
  ADD COLUMN csrf_token_hash BYTEA NOT NULL CHECK (octet_length(csrf_token_hash) = 32);
