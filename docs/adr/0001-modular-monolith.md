# ADR 0001: Start as a modular monolith

- Status: Accepted
- Date: 2026-07-26

## Decision

MIZ starts with one SvelteKit web application, one Rust/Axum API, and PostgreSQL. Caddy exposes one origin and proxies `/api/*` to the API. Authentication, authorization, and business rules live in the Rust API.

The API is split by responsibility inside one crate. REST serves normal operations, SSE will serve timeline updates, and WebSocket support is deferred until chat. Background work uses the PostgreSQL `jobs` table.

## Consequences

There is no Kafka or microservice boundary in the MVP. A service is extracted only after measured scaling or ownership pressure justifies it.
