UPDATE handles SET value = lower(value);

ALTER TABLE handles
    DROP CONSTRAINT handles_value_check,
    ADD CONSTRAINT handles_value_check CHECK (
        value ~ '^[a-z0-9][a-z0-9_]{1,22}[a-z0-9]$'
        AND value !~ '__'
    );

ALTER TABLE users
    ADD COLUMN handle_changed_at TIMESTAMPTZ,
    ADD CONSTRAINT users_bio_length CHECK (char_length(bio) <= 160);

ALTER TABLE sessions ADD COLUMN authenticated_at TIMESTAMPTZ;
UPDATE sessions SET authenticated_at = created_at;
ALTER TABLE sessions
    ALTER COLUMN authenticated_at SET NOT NULL,
    ALTER COLUMN authenticated_at SET DEFAULT now(),
    ADD CONSTRAINT sessions_authenticated_before_expiry CHECK (authenticated_at <= absolute_expires_at),
    ADD CONSTRAINT sessions_device_name_length CHECK (char_length(device_name) <= 100);
