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
cargo check
pnpm install
pnpm check
pnpm build
```
