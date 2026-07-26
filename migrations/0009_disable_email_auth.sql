ALTER TABLE auth_challenges
    DROP CONSTRAINT auth_challenges_kind_check,
    ADD CONSTRAINT auth_challenges_kind_check CHECK (kind = 'google_oauth') NOT VALID;

ALTER TABLE auth_identities
    DROP CONSTRAINT auth_identities_provider_check,
    ADD CONSTRAINT auth_identities_provider_check CHECK (provider = 'google') NOT VALID;
