# Password authentication for the MVP

## Context

The original plan used Magic Link email verification. The MVP must no longer depend on email delivery, and should use either username/password or Passkeys.

## Decision

Use case-insensitive username and password authentication for the MVP. Store only Argon2id hashes with random salts. Registration creates the user, credential, and first server-managed session atomically. Login errors do not reveal whether a username exists. Registration and login require same-origin requests and are rate-limited.

## Alternatives

- Passkeys: better phishing resistance, but require additional WebAuthn enrollment and recovery flows.
- Magic Link: rejected because email authentication is removed.

## Consequences

The MVP has no email address or account-recovery flow. Passkeys and recovery can be added post-MVP without changing existing session semantics. Migration `0008_password_auth.sql` removes the temporary email-registration tables and adds password credentials; migration `0009_disable_email_auth.sql` rejects new Magic Link identity and challenge records. Existing Magic Link accounts cannot log in with passwords without a later migration or recovery flow.

## Compatibility and migration

Deploy migration 0008 before the API. Clients must replace the multi-step Magic Link flow with `POST /api/v1/registrations` and `POST /api/v1/auth/login`.
