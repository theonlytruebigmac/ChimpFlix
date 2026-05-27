# V2 API — Shape, Versioning, Type Safety

> Status: **RFC skeleton.** Defines V2's HTTP API surface.

## Scope

The contract between V2's frontend and backend. Versioning, transport,
type-safety strategy, error model, auth conventions.

## V1 starting point

- REST under `/api/v1`.
- Hand-typed in TypeScript (`web/src/lib/chimpflix-api.ts`).
- Cookie auth (HMAC-signed sessions).
- SSE for realtime under `/api/v1/events`.
- Error model: `{ "error": { "code": "...", "message": "..." } }`.

## Open: API style

Three viable options:

- **REST (V1 carry-forward).** Familiar, browser-debuggable,
  cacheable. Cost: hand-keeping the TypeScript client in sync with
  the Rust handlers. V1's `chimpflix-api.ts` is 1500+ lines of
  manually-maintained types.
- **Typed RPC (tRPC-style, but Rust-native).** Compile-time-shared
  types via codegen from Rust schema to TypeScript. Excellent DX.
  Less curl-friendly. Tooling needs to be built or adopted.
  Candidates: `axum-typed-routing` + `ts-rs` for shared types;
  or build minimal codegen using Rust's `JsonSchema`.
- **GraphQL.** Strong type safety, frontend chooses what to fetch.
  Adds a layer of complexity (resolver dataloaders, N+1 prevention).
  For a single-frontend-single-backend scenario, GraphQL's value
  prop weakens. Likely overkill.

Recommendation: **typed RPC with codegen.** End-to-end types are a
hard requirement under V2 goals. REST works but pays its cost every
time we add an endpoint. GraphQL solves problems we don't have.

Concrete shape:

- Rust handler functions take typed request structs and return typed
  response structs.
- A build step generates the TypeScript client from the Rust types
  (`ts-rs` or similar).
- Routes still mount under `/api/v2/...` (HTTP-friendly), just with
  generated client wrappers.

## Carry forward

- Cookie session auth.
- Error envelope shape (`{ error: { code, message } }`).
- `/health` endpoint.
- Pagination conventions (`page`, `per_page`, response includes
  `total` or `next_cursor`).
- Admin routes gated by role hierarchy.
- Owner-only routes for destructive ops.

## What changes

- **Versioning.** `/api/v2`. The version is bumped on breaking
  changes within the V2 lifetime; minor changes don't bump the prefix.
- **Real-time.** SSE under `/api/v2/events` carries forward. See
  `REALTIME.md` for the event taxonomy.
- **Pagination.** Cursor-based by default (`?cursor=...`), offset-
  based for endpoints that need jump-to-page. V1 is offset-only;
  cursor is more robust for live-updating lists.
- **Idempotency keys.** For destructive POST/DELETE operations,
  optional `Idempotency-Key` header.

## Open questions

- **GraphQL revisit.** If the frontend grows multiple consumers (CLI,
  scripts, third-party scripts) or if data-shaping needs grow,
  revisit. Not for V2's initial cut.
- **WebSocket vs. SSE for realtime.** SSE works fine for one-way
  server-push, doesn't need WebSocket complexity. Stay on SSE unless
  a bidirectional need emerges (casting?).
- **API for third-party integrations.** Out of scope for V2 unless
  someone asks for it.

## Cut list

- **OAuth2 server.** ChimpFlix is not an identity provider; it is a
  consumer of Plex (and possibly Google). No outbound OAuth2.
- **Public read-only catalog API.** ChimpFlix is private.
- **GraphQL.** Rejected for V2; reconsider later if needs change.
