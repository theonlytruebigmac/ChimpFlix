<!-- Thanks for contributing! Keep the PR small and focused. -->

## What changed

<!-- One paragraph: the *why* before the *what*. Link an issue if there is one. -->

## How I tested

<!-- Bulleted checklist of what you verified locally. Include the command output
     when relevant (lint, typecheck, the test that exercises the change). -->

- [ ] `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `npm run lint && npm run typecheck && npm run build` (if `web/` changed)
- [ ] Smoke-tested the change in a browser (if user-visible)

## Screenshots / recordings

<!-- For UI changes, include before/after screenshots or a short recording.
     Delete this section if not applicable. -->

## Migrations

<!-- If this PR adds a new `.sql` migration in `crates/library/migrations/`,
     answer below. Otherwise delete this section. -->

- [ ] Migration is **forward-only** (no `DOWN` block; reversed via a new
      migration if ever needed).
- [ ] Migration is **idempotent** under `sqlx migrate run` (re-runs on a
      partially-applied DB don't error).
- [ ] Migration has been tested against a snapshot of the previous
      release's DB (or this PR adds the snapshot test).

## Breaking changes

<!-- Pre-1.0 we tolerate breaking changes, but they need a CHANGELOG entry
     and operator-facing release notes. If this PR breaks something, describe
     the migration path here. -->

## Checklist

- [ ] `CHANGELOG.md` updated under `[Unreleased]` (omit for chore / docs).
- [ ] No new `unwrap()` / `expect()` in request handler paths.
- [ ] No secrets, tokens, or PII in code, tests, or fixtures.
- [ ] If this PR touches auth, rate limiting, or transcoder spawn, the
      relevant entry in
      [docs/PUBLIC_RELEASE_HARDENING.md](../docs/PUBLIC_RELEASE_HARDENING.md)
      has been considered.
