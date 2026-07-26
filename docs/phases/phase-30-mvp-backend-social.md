# Phase 30: MVP Backend Social

- Status: Implemented
- Source tasks: [Reply](https://app.todoist.com/app/task/6h7ghvxFFrG27VCf), [Follow](https://app.todoist.com/app/task/6h7hCcxr8Qxq8pr7), [Following timeline](https://app.todoist.com/app/task/6h7hCjcjCVwGRV37), [Unfollow](https://app.todoist.com/app/task/6h7hCmxG8CQfHm47)
- Supporting source: [`dr.md`](../../dr.md), “短文投稿・Reply” and “Follow・承認・ホームタイムライン”

## Goal

Authenticated users can reply to visible posts, follow public accounts immediately, request access to private accounts, manage follow requests, and read a stable Following timeline containing only currently visible posts from themselves and accepted follows.

## Scope

### Reply

- Create a text-only Reply to a visible, published root post.
- Reuse Phase 20 content validation: 1–500 Unicode grapheme clusters after trimming, at most 8192 UTF-8 bytes, no blank content, and embedded newlines preserved.
- Require `Idempotency-Key` and apply the same actor, endpoint, key, and request-content binding as Post creation.
- Reject replies when the parent does not exist, is deleted, or is not visible to the requester, without revealing an inaccessible private parent.
- A Reply must not be visible to a broader audience than its parent. Reads must validate both the Reply author’s visibility and the parent Post’s visibility; `effectiveVisibility` alone must not grant access.
- List replies in `createdAt ASC, id ASC` order using keyset pagination.
- Existing Phase 20 `GET`, `PATCH`, and `DELETE /api/v1/posts/{postId}` behavior applies to Reply IDs, including author-only mutation, the 15-minute edit window, optimistic concurrency, edit history, and tombstone deletion.
- Nested replies are not added in this phase; a Reply targets a root Post.

### Follow and follow requests

- Reject self-follow.
- Following a public account creates or transitions the relationship to `accepted`.
- Following a private account creates or transitions the relationship to `pending`.
- Repeating a follow operation returns the current relationship without creating duplicates.
- The target user may accept or reject a pending request. Other users receive a non-enumerating not-found response.
- Unfollow is idempotent: `pending` becomes `cancelled`, `accepted` becomes `removed`, and repeated removal does not change counts or create rows.
- A later follow operation may transition `cancelled`, `rejected`, or `removed` back to the state required by the target’s current privacy.
- Followers, following, and follow-request reads include accepted or pending relationships as appropriate and never expose inaccessible private user data.
- Follower and following counts are derived from accepted relationships rather than stored counters, preventing duplicate or stale counts.

### Following timeline

- Return posts authored by the current user and by users with an `accepted` follow relationship.
- Include only `published` posts currently visible to the requester; exclude deleted posts, tombstones, and replies from the top-level timeline.
- Re-evaluate relationship state, account privacy, and post visibility on every request. Phase 30 does not cache timeline authorization results.
- Sort by `createdAt DESC, id DESC` and use keyset pagination. Offset pagination is prohibited.
- Default to 30 items per page and cap an explicit `limit` at 100.
- Return an opaque HMAC-SHA256-signed Base64url cursor containing the final item’s sort keys and an expiry. Cursors expire after 24 hours and invalid, modified, or expired cursors fail with stable Problem Details codes.
- Newly committed posts are immediately eligible for the next timeline request, satisfying the five-second availability requirement without realtime transport.
- Recommendation scores are not mixed into the Following timeline.

## Explicitly out of scope

- Attachments and media Replies.
- Nested Reply trees.
- Block and Mute behavior, which `dr.md` assigns to post-MVP hardening. Phase 30 queries must remain straightforward to extend with those filters later.
- Reactions, notifications, search, recommendations, and federation.
- SSE, WebSocket, automatic insertion of new posts, and the frontend “new posts” button. Clients poll or refresh the HTTP timeline in the MVP.
- Cached or fan-out-on-write timelines and stored follower counters.

## Dependencies

- Phase 00 object IDs, Post/Reply schema, follow relationship schema, API conventions, and database constraints.
- Phase 10 authenticated sessions, CSRF protection, user privacy, authorization, and non-enumerating private-resource behavior.
- Phase 20 Post content validation, idempotency storage, visibility checks, editing, deletion, and tombstones.
- A secret `CURSOR_SIGNING_KEY` environment variable is required by the API. It must not be logged or committed and must be consistent across API instances while issued cursors remain valid.
- PostgreSQL migrations, OpenAPI, generated API client, environment documentation, and tests must change together with the implementation.

## Contract work

Update the canonical OpenAPI document for:

- `POST /api/v1/posts/{postId}/replies`
- `GET /api/v1/posts/{postId}/replies`
- `PUT /api/v1/users/{targetUserId}/follow`
- `DELETE /api/v1/users/{targetUserId}/follow`
- `GET /api/v1/follow-requests`
- `POST /api/v1/follow-requests/{relationshipId}/accept`
- `POST /api/v1/follow-requests/{relationshipId}/reject`
- `GET /api/v1/users/{userId}/followers`
- `GET /api/v1/users/{userId}/following`
- `GET /api/v1/timelines/home`

Use `/api/v1`, camelCase JSON, RFC 3339 UTC timestamps, opaque public IDs, and `application/problem+json`. Request and response schemas, cursor fields, pagination limits, and all failure responses must be explicit.

## Stable errors

Use existing shared errors where applicable and add the following codes to the canonical contract:

- `cannot_follow_self`
- `invalid_state_transition`
- `parent_not_visible`
- `invalid_cursor`
- `cursor_expired`
- `target_not_visible`

Inaccessible users, posts, replies, and follow requests must not reveal whether the resource exists.

## Acceptance criteria

1. A valid Reply is created idempotently with Phase 20 content rules and cannot outlive or exceed the parent Post’s visibility.
2. An invisible, deleted, tombstoned, or non-root parent is rejected without private-resource enumeration.
3. Reply listing is stable across pages and author-only edit/delete behavior remains covered by tests.
4. Following a public account yields `accepted`; following a private account yields `pending`; self-follow fails.
5. Only the private target may accept or reject a pending request, and invalid transitions fail predictably.
6. Follow and unfollow retries are idempotent, relationship rows remain unique, and derived counts remain correct.
7. The Following timeline contains only self and accepted-follow posts that are visible at read time, ordered by `createdAt DESC, id DESC`.
8. Timeline pages default to 30 items, have no duplicates or omissions for a stable dataset, and reject modified or expired cursors.
9. Follow, unfollow, privacy changes, and deletion affect the next timeline fetch without stale authorization cache results.
10. OpenAPI, generated client, migrations, domain tests, database tests, and authenticated HTTP tests cover the behavior and important failure paths.

## Implementation order

1. Finalize OpenAPI schemas and cursor configuration.
2. Add Reply creation/listing and parent visibility enforcement.
3. Add Follow state transitions, requests, lists, and derived counts.
4. Add the signed keyset cursor and Following timeline query.
5. Regenerate the API client and run the full repository CI suite against PostgreSQL.
