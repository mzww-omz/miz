# MIZ

MIZ is a social platform.

## Repository layout

- `apps/api`: Rust/Axum API
- `apps/web`: SvelteKit web application
- `packages/api-client`: generated OpenAPI client output
- `packages/ui`: shared UI and design tokens
- `migrations`: PostgreSQL migrations
- `openapi`: API contracts
- `infra`: deployment and operations configuration
- `docs/adr`: architecture decision records

## Local development

```sh
docker compose up --build
```

Open <http://localhost:8080>. The gateway serves the web app and forwards `/api/*`, `/healthz`, and `/readyz` to the Rust API on the same origin.

## Checks

```sh
pnpm install
pnpm run ci
```

Set `DATABASE_URL` to run the PostgreSQL integration test; CI supplies an ephemeral PostgreSQL 17 service. Database deployment is independent from API startup:

```sh
DATABASE_URL=postgres://... cargo run --bin miz-migrate
```

Environment templates live in `infra/environments`. Release and rollback steps are in `docs/runbooks/release.md`.
