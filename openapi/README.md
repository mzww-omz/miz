# OpenAPI

`openapi.yaml` is the authoritative HTTP contract. JSON fields use camelCase, timestamps use RFC 3339 UTC, and errors use RFC 9457 Problem Details.

Generate and check the TypeScript client with:

```sh
pnpm --filter @miz/api-client check
```

The Rust API serves the same document as JSON at `/openapi.json`.
