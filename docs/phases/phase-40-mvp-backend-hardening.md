# Phase 40: MVP Backend Hardening

- Status: Complete
- Source tasks: [Block / Mute](https://app.todoist.com/app/task/6h7hQpp8Wv4Q62C7), [Moderation Administration](https://app.todoist.com/app/task/6h7hfX6xhPf42qvf), [Account Privacy](https://app.todoist.com/app/task/6h7hQpfM24QvPhvf), [Post Report](https://app.todoist.com/app/task/6h7hPQG26gfpHJj7), [Account Delete](https://app.todoist.com/app/task/6h7h9W9wjrWxcM47), [30-day purge](https://app.todoist.com/app/task/6h7h9c9vr4fxfh9f), [30-day restore](https://app.todoist.com/app/task/6h7h9fMcFVhC9RV7)
- Supporting source: [`dr-2.md`](../../dr-2.md), “Phase 40 backend hardening”

## Goal

Establish the shared authorization, safety, moderation, audit, and account-deletion rules required before adding search, chat, federation, recommendations, or additional clients. Existing and future APIs must deny access consistently without exposing private resources.

## Scope

### Shared relationship policy

- Centralize server-side decisions for profile visibility, post visibility, Reply visibility, following, replying, and future reaction and direct-message eligibility.
- Evaluate in this order: account status, block relationship, parent Post visibility, author privacy, and accepted Follow relationship.
- Preserve the Phase 30 rule that a Reply is visible only when both its author and parent Post are visible to the requester.
- Return the same non-enumerating not-found response for missing and inaccessible users, Posts, Replies, relationships, and reports.
- Apply current authorization state on every request; do not cache authorization outcomes in this phase.
- Add a shared authorization matrix covering public/private accounts, accepted/pending follows, either-direction blocks, account suspension, and parent/Reply visibility.

### Account privacy hardening

- Reuse the existing `users.privacy`, `PATCH /api/v1/users/me`, Follow state machine, and Follow-request endpoints. Do not add duplicate privacy or Follow-request resources.
- Public profiles and published Posts are visible unless another policy, including a block or suspension, denies access.
- Private-account Posts and Replies are visible only to the author and accepted followers, subject to parent visibility.
- Changing from public to private preserves accepted followers and makes only new Follow attempts pending.
- Changing from private to public does not automatically accept existing pending requests.
- Reply counts and lists include only Replies visible to the requester.
- Profile, Post, Reply, Follow, mutation, and report operations all enforce authorization server-side.

### Block and Mute

- An authenticated user may block or unblock another user and mute or unmute another user. Self-block and self-mute are rejected.
- A block is directional for disclosure but restrictive in both directions: neither party may view the other’s profile or content, follow, create a Reply targeting the other party’s Post, or use future reaction or direct-message operations.
- Creating a block atomically transitions any Follow relationships in either direction to a non-accepted state.
- Block, unblock, mute, and unmute retries are idempotent and do not create duplicate rows.
- A block is not disclosed to the blocked user. Denied reads use the shared non-enumerating response.
- A mute preserves Follow relationships and affects only the muting user’s Following timeline. It does not change profile, Reply, or relationship visibility for either party.
- Search, notifications, reactions, and chat are not implemented in Phase 40. Their later implementations must call the shared relationship policy; Phase 40 does not add placeholder endpoints for them.

### Post reports

- An authenticated user may report a visible Post or Reply authored by another user.
- Reasons are `spam`, `harassment`, `hatefulContent`, `violence`, `sexualContent`, `illegalOrDangerousTrade`, `personalInformation`, `copyright`, or `other`.
- `other` requires a non-blank explanation of at most 500 Unicode grapheme clusters and 8192 UTF-8 bytes. Other reasons may include an explanation within the same limits.
- Store an immutable evidence snapshot containing the target ID and type, author ID, latest content and edit version, creation time, and existing attachment references at report time. Later edits or deletion must not alter the snapshot.
- Allow only one unresolved report per reporter and target. A retry returns the existing report; updating its reason or explanation uses optimistic concurrency.
- Report states are `received`, `inReview`, `actioned`, and `noAction`. Only documented transitions are allowed.
- Report counts alone never delete content. Automated emergency restriction is outside this phase.
- Rate-limit each reporter to 20 new reports per rolling hour and return `Retry-After` when exceeded.
- Retain resolved evidence for 180 days, then delete it with a deterministic maintenance job while preserving the minimum audit record required by policy.
- Notification delivery is deferred to the notification phase. Report status remains available through the authenticated API without exposing staff identity or internal deliberation.

### Moderation administration

- Use dedicated operator accounts, credentials, sessions, and role assignments; personal social accounts cannot be granted operator roles.
- Roles are `support`, `moderator`, `seniorModerator`, `administrator`, and `auditor`. Authorization is deny-by-default and grants only the documented permissions.
- All operator accounts require MFA. Store TOTP secrets encrypted at rest and hash one-time recovery codes; never log either. Administrator role changes and permanent suspensions require a recent MFA confirmation.
- Support may access basic account and support-case information but not private Posts, report evidence, chat content, or enforcement actions.
- Moderator may review reports, remove reported content, and apply temporary feature restrictions, but may not permanently suspend accounts or change operator roles.
- Senior Moderator may temporarily suspend accounts and review appeals. The original decision-maker cannot review the same appeal.
- Administrator alone may permanently suspend accounts and change operator roles.
- Auditor has read-only, time- and subject-bounded access to audit records and cannot perform moderation actions.
- Record operator sign-in, administrative reads, and every moderation action in an append-only audit log with actor, target, reason, before/after state, timestamp, request ID, and related report ID where applicable.
- Retain audit records for one year. Application roles, including administrators, cannot update or delete them.
- Do not include credentials, session tokens, unnecessary private messages, or unrelated personal data in audit records.
- Notify a different administrator and an auditor of high-privilege grants, removals, and permanent suspensions through a durable audit event. User-facing notification delivery remains out of scope until the notification phase.

### Account deletion and restoration

- An authenticated user may request account deletion after re-authentication. The request immediately revokes all sessions and logically disables the account.
- While deletion is pending, hide the profile and content from normal reads and reject login and state-changing operations.
- Store the deletion request and restoration deadline in UTC. The deadline is exactly 30 × 24 hours after the request is committed.
- A user may cancel or restore within the grace period after successful identity verification. Restoration atomically reactivates the profile, Posts, and Follow relationships and cancels unclaimed purge work.
- Once purge processing has atomically claimed an expired request, restoration fails predictably and cannot race the purge.
- After the grace period, a retry-safe background job removes credentials, sessions, private attributes, profile data, and existing attachment references.
- At final purge, published Posts and Replies lose content and author linkage. Keep a tombstone only where required to preserve an existing Reply thread; remove reactions when that feature exists.
- Chat is not implemented in Phase 40. Its future schema must support retaining participant-visible history while removing the deleted account’s identity.
- Purge jobs record operational status and failures without copying deleted personal data into logs or error payloads.

## Explicitly out of scope

- Search, notifications, reactions, chat, media processing, ActivityPub, recommendations, and their user interfaces.
- Block or Mute behavior on surfaces that do not yet exist. Those phases must integrate with the shared relationship policy before release.
- Automated moderation based only on report volume.
- General customer-support case management.
- A browser administration interface; Phase 40 provides authenticated administration APIs only.
- Changes to Phase 30 pagination, content limits, or timeline ordering except the required Block and Mute filters.

## Dependencies

- Phase 10 authenticated user sessions, CSRF and Origin validation, rate limiting, profile privacy, and non-enumerating authorization behavior.
- Phase 20 Post and Reply persistence, edit history, optimistic concurrency, tombstones, and idempotency storage.
- Phase 30 Follow state transitions, Follow requests, Reply visibility, and Following timeline.
- PostgreSQL migrations, OpenAPI, generated API client, environment documentation, and tests must change together with implementation.
- New secrets for operator MFA encryption must be supplied through deployment configuration, remain consistent across API instances, and never be committed or logged.
- Purge and retention jobs require a single-claim, retry-safe worker execution path. They must not run as uncontrolled work spawned from HTTP handlers.

## Data and migration work

Reuse `users.privacy`, `follow_relationships`, `posts`, `post_edits`, and existing session and idempotency tables. Add only the tables needed for new behavior:

- `user_blocks` with `(blocker_id, blocked_id)` uniqueness and a no-self constraint.
- `user_mutes` with `(muter_id, muted_id)` uniqueness and a no-self constraint.
- `content_reports` with reporter, target, reason, explanation, state, version, timestamps, and an unresolved-report uniqueness constraint.
- `content_report_evidence` containing the immutable evidence snapshot and retention deadline.
- `operator_accounts`, `operator_credentials`, `operator_sessions`, `operator_mfa_factors`, and `operator_role_assignments`.
- `moderation_actions` and append-only `audit_log_entries`.
- `account_deletion_requests` with explicit `pending`, `cancelled`, `purging`, `purged`, and `restored` states.
- `maintenance_jobs` for purge and retention work with retry and claim metadata.

Use database constraints for finite states, no-self relationships, uniqueness, and valid deletion timestamps. Use transactions for block creation, moderation actions, deletion requests, restoration, and purge claims.

## Contract work

Update the canonical OpenAPI document for:

- `POST /api/v1/users/{targetUserId}/block`
- `DELETE /api/v1/users/{targetUserId}/block`
- `POST /api/v1/users/{targetUserId}/mute`
- `DELETE /api/v1/users/{targetUserId}/mute`
- Existing `PATCH /api/v1/users/me` privacy behavior
- `POST /api/v1/posts/{postId}/reports`
- `GET /api/v1/reports/{reportId}` for the reporting user’s status view
- `PATCH /api/v1/reports/{reportId}` for an unresolved report’s reason or explanation
- `POST /api/v1/users/me/deletion-requests`
- `POST /api/v1/users/me/deletion-requests/current/cancel`
- `POST /api/v1/users/me/deletion-requests/current/restore`
- Operator authentication and MFA enrollment, challenge, recovery, and session-revocation endpoints
- Operator report queue, review, appeal, enforcement, role-assignment, and audit-read endpoints under `/api/v1/admin`

Every state-changing route requires CSRF protection or the separate operator-session equivalent, explicit request and response schemas, stable Problem Details responses, and server-side authorization. Administrative schemas must not expose private evidence or audit fields to public user schemas.

## Stable errors

Reuse shared errors where applicable and add explicit contract entries for:

- `cannot_block_self`
- `cannot_mute_self`
- `cannot_report_own_content`
- `report_reason_required`
- `report_already_exists`
- `report_not_editable`
- `invalid_report_transition`
- `operator_auth_required`
- `operator_mfa_required`
- `operator_mfa_stale`
- `operator_permission_denied`
- `appeal_reviewer_conflict`
- `deletion_already_pending`
- `deletion_not_pending`
- `restoration_window_expired`
- `purge_in_progress`
- `invalid_state_transition`

Inaccessible targets use the existing non-enumerating not-found code instead of revealing block, report, suspension, or deletion state.

## Acceptance criteria

1. All existing profile, Post, Reply, Follow, Follow-request, and timeline APIs use one shared authorization policy and pass the authorization matrix.
2. Block creation atomically removes accepted or pending Follow access in both directions and immediately affects subsequent reads and writes.
3. Block, unblock, mute, and unmute are idempotent, unique, non-notifying, and protected against self-targeting.
4. A mute removes only the target author’s top-level Posts from the muting user’s Following timeline while preserving Follow state and other visibility.
5. Privacy changes preserve accepted followers, do not auto-accept pending requests, and never expose invisible Reply existence through lists or counts.
6. A visible Post or Reply can be reported with validated reasons, immutable evidence, unresolved-report deduplication, optimistic concurrency, and the required rate limit.
7. Operator roles enforce least privilege, operator accounts are separate from social accounts, all operator access requires MFA, and high-risk actions require recent MFA confirmation.
8. Administrative reads and actions create immutable, privacy-minimized audit entries retained for one year; report evidence is removed after its 180-day retention period.
9. Account deletion immediately revokes access, allows verified restoration for 30 days, and rejects restoration after a purge job atomically claims the request.
10. Purge and retention jobs are retry-safe, do not duplicate side effects, do not leak deleted data, and are covered by PostgreSQL integration tests.
11. Unauthorized and inaccessible resources return stable, non-enumerating Problem Details responses across direct URLs and collection endpoints.
12. OpenAPI generation leaves no diff, migrations apply successfully twice, and domain, database, authenticated HTTP, security-regression, and job-concurrency tests cover the important paths.

## Implementation order

1. Finalize the authorization matrix, OpenAPI contracts, operator security model, and migration design.
2. Extract the shared relationship policy and add privacy regression tests without changing existing successful behavior.
3. Add Block and Mute persistence, state transitions, timeline filtering, and HTTP endpoints.
4. Add reports, evidence snapshots, rate limiting, and retention jobs.
5. Add dedicated operator authentication, MFA, roles, report review, moderation actions, and append-only audit logging.
6. Add deletion requests, logical disablement, restoration, purge claiming, and retry-safe purge processing.
7. Regenerate the API client and run the complete CI suite against PostgreSQL, including migration replay and concurrency tests.
