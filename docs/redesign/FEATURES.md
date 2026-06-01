# Gap features — full-stack build plan

The mockup-only features, made real. Backend = Rust workspace (`crates/server`, `crates/library`
migrations + `queries.rs`, SQLite). Build each on its own, leaving `cargo build` + `next build` green.
Source plan: feasibility workflow (backend mapped per feature).

## Tier 1 — build now (contained, hook into existing infra)

- **Trakt token-expiry warning** — S (~½ day). Token row already stores `expires_at`; just compute
  `expiring_soon` into the integrations response + surface in the Trakt UI. No migration.
- **Hide watched from Continue Watching** — S (~1 day). Add a user pref; filter the on-deck query.
  Decision: "hide 100% watched" (already effectively done) vs "show only unstarted" → going with a
  "Only show new items" toggle (hide partially-watched).
- **Home rails on/off + reorder** — M (2–3 days). Persist a per-user rail config (order + enabled);
  home materialization respects it. Fixed set of today's rails; add/remove rails is future work.
- **Notification preferences framework + quiet hours** — M (2–3 days). New `users.notification_prefs_json`;
  `should_notify_user(kind, now)` guard in `notifier.rs` (per-kind enable, per-channel, quiet-hours
  window, UTC v1). Wires the existing kinds (user.registered / 2fa.disabled / job.failed).

## Tier 2 — build, but needs a product decision first

- **Personal content notifications** ("new episode in a show I watch", "new movie in my libraries")
  — M+. Backend emits ONLY admin notifications today; these need NEW per-user content-detection
  events (on scan/enrich → notify watchers). This is the part of the Notifications page beyond the
  framework above. **Decision: build these events now, or framework-only first?**
- **Discord per-user routing** — M–L (3–5 days). Follows the Trakt per-user-credential pattern; store
  a per-user webhook URL, POST embeds from the notifier. Build after the notification framework.
- **3-state access Full / View / None** — L (3–5 days). Authorization refactor: migration + every
  access-check call site. **Decision: what does "View" permit?** (recommend: browse metadata, no play.)
- **Kids-safe browsing** — M (4–5 days), **blocked on data**: needs content age-ratings on items +
  a per-user maturity limit + filtering in browse/home queries. **Decision: rating standard + how
  sparse is the rating data?**

## Tier 3 — defer (out of scope for a batch)

- **Watch-party invites + sync** — **XL, ~3–4 weeks.** New per-party WS rooms (refactor `/api/v1/ws`
  from global), shared playback-position broadcast, clock-skew/latency sync, leader/pause control,
  room-code/invite flow. A project of its own, not a batch item. Quiet-hours-skip rides on it.
- **Simkl** — trivial UI placeholder ("coming soon"); no backend until a real Simkl integration.

## Build order (once decisions are in)
Trakt-expiry → hide-watched → home-rails → notification-framework → (content-notifications) →
Discord → 3-state-access → kids-safe. Watch-party deferred. Simkl placeholder anytime.
