# AGENTS.md

## Purpose

This file contains repository-wide rules that apply to every development phase of MIZ.

Phase-specific requirements belong in the relevant documents under `docs/phases/`. Do not duplicate detailed product specifications here.

When instructions conflict, use this precedence:

1. Explicit user instructions for the current task
2. This `AGENTS.md`
3. The relevant phase document
4. Other repository documentation
5. Existing implementation patterns

Do not silently resolve contradictory requirements. Use the smallest implementation consistent with the current phase and document important decisions.

## Scope Discipline

- Work only within the requested phase and task.
- Do not add unrelated features.
- Do not expand the MVP without explicit approval.
- Do not implement post-MVP functionality during MVP work.
- Avoid speculative abstractions and premature generalization.
- Prefer the smallest coherent change that satisfies the requirement.
- Do not perform large refactors unless required for the task.
- Do not rewrite unrelated code during focused work.

Before implementation, read the relevant phase document and confirm that the requested work belongs to that phase.

## Architecture

- Use the documented stack and repository structure.
- Keep HTTP handlers thin.
- Keep business rules outside transport-layer code.
- Keep database access in repository or infrastructure modules.
- Keep domain logic independent from Axum request and response types.
- Maintain clear boundaries between domains and infrastructure.
- Avoid circular dependencies.
- Create shared utilities only when they are genuinely shared.
- Prefer explicit code over complex generic frameworks.
- Do not replace major frameworks, databases, authentication systems, or dependencies without explicit approval.

## API Contracts

OpenAPI is the canonical HTTP API contract.

For every endpoint change:

- Update the OpenAPI document in the same change.
- Keep request and response schemas explicit.
- Use consistent naming and error behavior.
- Do not invent undocumented fields or endpoints.
- Do not expose internal implementation details.

API conventions:

- Prefix API routes with `/api/v1`.
- Use `camelCase` for JSON fields.
- Use RFC 3339 UTC timestamps.
- Use `application/problem+json` for API errors.
- Use stable machine-readable error codes.
- Avoid revealing whether inaccessible private resources exist.

## Data and Database

- Use PostgreSQL migrations for every schema change.
- Keep migrations deterministic and committed.
- Use database constraints for uniqueness and finite states where appropriate.
- Use explicit foreign keys.
- Use transactions for multi-step operations that must succeed atomically.
- Use parameterized SQL.
- Add indexes based on real access patterns.
- Avoid premature denormalization.
- Do not rely only on application code to enforce uniqueness.
- Consider deployment and compatibility impact before changing schemas.
- Never store raw authentication or verification secrets when a secure hash is sufficient.

## Security and Privacy

Security requirements are mandatory.

- Deny access by default.
- Validate authentication and authorization separately.
- Enforce authorization and visibility on the server.
- Validate all untrusted input.
- Protect state-changing requests against CSRF.
- Validate OAuth state, PKCE, redirect targets, and callback data.
- Rate-limit authentication and registration endpoints.
- Hash sensitive tokens.
- Use secure randomness where required.
- Use constant-time comparison where relevant.
- Keep public profile data separate from private user data.
- Avoid account, email, and private-resource enumeration.
- Do not weaken security checks to make tests pass.

Never commit or log:

- Passwords
- Session tokens
- Magic Link tokens
- OAuth authorization codes
- CSRF secrets
- Access tokens
- Private keys
- Production credentials
- Real user personal data

Development placeholders must be clearly marked as insecure.

## Error Handling

- Use typed errors.
- Convert internal errors into stable API errors at the transport boundary.
- Do not expose stack traces, SQL errors, secrets, or internal paths.
- Avoid panics in normal request paths.
- Do not use `unwrap()` or `expect()` in request handling or infrastructure code unless the condition is proven impossible and documented.
- Log enough context to diagnose failures without logging sensitive data.
- Preserve consistent status codes and Problem Details shapes.

## Concurrency and Idempotency

- Use optimistic concurrency where concurrent updates matter.
- Reject stale updates instead of silently overwriting newer data.
- Use idempotency for create operations that may be retried.
- Bind idempotency records to the authenticated actor, endpoint, key, and request content.
- Reusing a key with different request content must fail.
- Keep state transitions explicit and validate allowed transitions.

## Logging and Observability

Use structured logs.

Include where appropriate:

- Request ID
- Route
- Method
- Status
- Duration
- Authenticated user ID
- Stable error code

Do not log request bodies or headers containing sensitive information. Health checks must not expose configuration or secrets.

## Code Quality

- Use descriptive names.
- Keep functions and modules focused.
- Avoid hidden global mutable state.
- Remove dead code.
- Do not leave unexplained TODOs.
- Add comments for decisions and non-obvious constraints, not obvious syntax.
- Keep public APIs narrow.
- Prefer readable code over clever code.
- Avoid adding dependencies for trivial helpers.
- Do not introduce mock, placeholder, or in-memory persistence into production paths.

Rust:

- Keep domain errors separate from HTTP errors.
- Use newtypes for important identifiers where practical.
- Use transactions for atomic workflows.
- Prefer compile-time checked SQL when supported.

TypeScript:

- Keep strict mode enabled.
- Avoid `any`.
- Validate external data at boundaries.
- Do not treat frontend validation as a security boundary.
- Do not duplicate backend business logic as an independent source of truth.

## Testing

Every behavior change requires tests at the appropriate level.

Use:

- Unit tests for domain rules
- Integration tests for database behavior
- HTTP tests for API contracts
- Regression tests for security-sensitive behavior
- Migration tests for schema changes

Run the relevant repository commands, including where applicable:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm ci
npm run check
npm run test
npm run build
```

Do not remove or weaken tests without explaining the behavior change. Do not claim that a check passed unless it was actually run.

## Documentation

Update documentation when changing:

- API contracts
- Database schemas
- Authentication or authorization behavior
- State machines
- Environment variables
- Deployment behavior
- Phase scope

Record material decisions under:

```text
docs/decisions/NNNN-short-title.md
```

A decision record should contain context, decision, alternatives, consequences, and compatibility or migration impact.

Use English for repository technical documentation unless explicitly instructed otherwise.

## Agent Workflow

Before editing:

1. Read this file.
2. Read the relevant phase document.
3. Inspect the existing implementation.
4. Identify affected contracts, migrations, tests, and security boundaries.
5. Confirm that the task belongs to the requested phase.

During implementation:

1. Make the smallest coherent change.
2. Preserve documented behavior.
3. Update contracts and migrations when required.
4. Add or update tests.
5. Run relevant checks.
6. Review security, privacy, and compatibility impact.

After implementation:

1. Summarize changed behavior.
2. List important files changed.
3. Report commands run and their results.
4. State unverified assumptions.
5. State remaining risks or follow-up work.
6. Do not claim success for anything not verified.

## Prohibited Behavior

Agents must not:

- Expand scope without approval.
- Implement features from another phase without approval.
- Invent requirements, endpoints, fields, or state transitions.
- Replace the documented stack without approval.
- Trust client-provided authorization or visibility decisions.
- Store raw sensitive tokens.
- Disable security checks for convenience.
- Hide failing tests.
- Claim tests passed when they were not run.
- Commit secrets or personal data.
- Make unrelated refactors during focused work.
- Add speculative infrastructure without a current requirement.

## Definition of Done

A task is complete only when:

- The requested behavior is implemented.
- The change stays within the requested phase.
- Contracts and migrations are updated when applicable.
- Authorization and validation are enforced server-side.
- Errors follow repository conventions.
- Tests cover the behavior and important failure paths.
- Relevant formatting, linting, tests, and builds pass.
- Documentation is updated.
- No secrets or private data are exposed.
- Assumptions and remaining risks are reported honestly.
