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

## Local checks

```sh
cargo check
pnpm install
pnpm check
```
