# Release and rollback

## Environment order

Changes move through `test` → `staging` → `production`. Never deploy a mutable tag. Configure GitHub `staging` and `production` environments with required reviewers.

## Release

1. Run the CI workflow and require every job to pass.
2. Build and push API and web images once with an immutable tag.
3. Deploy that tag to staging.
4. Run `miz-migrate` as a one-shot job before starting the new API.
5. Verify `/healthz`, `/readyz`, `/openapi.json`, login, one authenticated read, and one authenticated write.
6. Check error rate, latency, logs, and `miz_http_requests_total` during a limited canary.
7. Promote the exact staging image digests to production after approval.
8. Repeat migration and smoke checks; stop rollout on any failure.

## Migration rule

Migrations are forward-only and must remain compatible with both the current and immediately previous application release. Use expand/migrate/contract across separate releases; do not rename, drop, or narrow a live column in the same release that changes application usage.

## Rollback

1. Stop the rollout.
2. Restore the immediately previous API and web image digests; do not reverse a committed migration.
3. Verify readiness and the smoke checks above.
4. If a forward data fix is required, ship a new migration after backup verification.
5. Record the incident, affected tag, migration version, and recovery timestamps.
