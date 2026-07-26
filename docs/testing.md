# Test policy

Every leaf feature must add the smallest runnable checks covering its applicable outcomes:

- success
- unauthenticated (`401`)
- unauthorized (`403`)
- missing resource (`404`)
- conflict or stale version (`409`/`412`/`428`)
- rate limit (`429` with `Retry-After`)

Unit tests cover domain rules. Integration tests use an ephemeral PostgreSQL database and exercise migrations and SQL constraints. Contract generation must leave no diff. Release smoke tests run against staging before production promotion.
