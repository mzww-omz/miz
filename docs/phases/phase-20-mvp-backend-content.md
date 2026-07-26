# Phase 20: MVP Backend Content

- Status: Implemented
- Source tasks: [Post](https://app.todoist.com/app/task/6h6mV5HHqQ9fRChf), [Short post](https://app.todoist.com/app/task/6h7ghmRJ6w3g8hM7), [Post Edit / Delete](https://app.todoist.com/app/task/6h7hQpf48cFhqCq7)
- Supporting source: [`dr.md`](../../dr.md), “短文投稿・Reply”

## Goal

Authenticated users can create a short text post and can edit or delete only their own posts. This phase preserves the MVP’s content validation, optimistic concurrency, authorization, and audit requirements.

## Scope

### Create a short post

- Create an authenticated post containing 1–500 Unicode grapheme clusters after trimming leading and trailing whitespace.
- Reject empty or whitespace-only content and content over 500 grapheme clusters.
- Preserve embedded newline characters.
- Treat post content as untrusted text: render it without XSS.
- Require an `Idempotency-Key` for creation; bind a stored result to the authenticated actor, endpoint, key, and request content. Reuse with different content must fail.
- Return a stable API error for validation and creation failures.

### Edit and delete

- Only the author may edit or delete a post.
- Permit edits only for 15 minutes after creation.
- Require `If-Match` for edits and reject stale versions.
- Mark an edited post and retain its final edit time.
- Save pre- and post-edit content, attachment references when applicable, and edit time as an operator-only audit history.
- Allow the author to delete at any time.
- If a deleted post has no child replies, remove it from normal timeline and thread results. If it has child replies, retain only a tombstone so its children remain intelligible.
- Do not expose edit history or report evidence to ordinary users.

## Explicitly out of scope

- Media upload, storage, processing, and attachment-only posts.
- Reply creation and reply listing: Todoist assigns `Reply` to Phase 30.
- Home timeline fan-out, follow visibility, recommendations, notifications, search, reactions, and moderation UI.

## Dependencies

- Phase 00 data model and shared API/error infrastructure.
- Phase 10 authenticated session, CSRF, authorization, and profile privacy foundations.
- PostgreSQL migrations, OpenAPI, and generated API client must change together with any endpoint or schema change.
- Reply-aware deletion requires the Phase 00 post relation schema; it must not create Reply APIs ahead of Phase 30.

## Contract work

Before implementation, update the canonical OpenAPI document for the Phase 20 post endpoints described in `dr.md`:

- `POST /api/v1/posts`
- `GET`, `PATCH`, and `DELETE /api/v1/posts/{postId}`

Use `/api/v1`, camelCase JSON, RFC 3339 UTC timestamps, and `application/problem+json`. Keep HTTP handlers thin; domain rules and database access remain outside handlers.

## Acceptance criteria

1. A valid authenticated request creates a text post of 1–500 grapheme clusters and preserves newlines.
2. Empty, whitespace-only, and over-limit content fail predictably; emoji and combined characters are counted as grapheme clusters.
3. Repeating a create request with the same idempotency key and content returns the original result; changing content with that key fails.
4. Only the author can edit or delete; edits after 15 minutes and stale `If-Match` values are rejected.
5. A successful edit exposes edited state and last-edit time, while the edit history remains operator-only.
6. Deleting a leaf post removes it from normal reads; deleting a post with child replies returns a tombstone and retains no author content or reactions.
7. The OpenAPI contract, generated client, migration, HTTP tests, and database tests cover the implemented behaviour and failure paths.

## Resolved scope decisions

1. Attachments are post-MVP as specified by `dr.md`; Phase 20 accepts text-only posts.
2. Home timeline visibility is verified in Phase 30. Phase 20 persists posts immediately but does not add a timeline endpoint.
3. Reply creation, listing, editing, and deletion belong to Phase 30. Phase 20 keeps deletion tombstone semantics compatible with the existing reply relation.
4. Phase 20 retains post edit history outside public responses and records deletions in the security audit log. Report evidence retention begins with the later reporting/moderation phase because no report exists in Phase 20.
