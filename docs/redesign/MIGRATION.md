# Settings & Admin redesign — production migration

Branch: `redesign/settings-admin-ia`. Working **in place** under `/settings`, full IA
restructure. Keep `next build` / `tsc --noEmit` green at every phase boundary. Operator commits.

Design source of truth: the static mockups in `docs/redesign/*.html` (open `index.html`).
Stack: Next 16 (app router), React 19, Tailwind v4 (`@theme` tokens in `globals.css`,
`bg-(--color-accent)` syntax, registered `accent` color), no `cn` helper (template strings).

## New route tree (`/settings`)

**You** (`requireUser`): `account` · `playback` · `integrations` · `notifications` · `devices` · `home`
**Server** (`requireOwner` at the admin layout — unchanged):
`admin/overview` · `admin/activity` · `admin/libraries` (tabs: Settings·Collections·Agents·Optimized)
· `admin/tasks` (Overview·Queue·Activity·Flow) · `admin/transcoding` (Engine·Presets·Pre-roll)
· `admin/users` (Users·Access·Groups·Devices·Invites) · `admin/network` · `admin/notifications`
(Email·Webhooks) · `admin/credentials` · `admin/maintenance` (Cleanup·Health·Backups·Bulk)
· `admin/logs` (Server logs·Audit) · `admin/general`

## Redirect map (next.config.ts `redirects()`, permanent)

```
/settings/player                              -> /settings/playback
/settings/libraries                           -> /settings/home
/settings/admin                               -> /settings/admin/overview
/settings/admin/status/stats                  -> /settings/admin/activity
/settings/admin/status/alerts                 -> /settings/admin/overview
/settings/admin/library                       -> /settings/admin/libraries
/settings/admin/library/libraries             -> /settings/admin/libraries?tab=settings
/settings/admin/library/collections           -> /settings/admin/libraries?tab=collections
/settings/admin/library/agents                -> /settings/admin/libraries?tab=agents
/settings/admin/library/versions              -> /settings/admin/libraries?tab=optimized
/settings/admin/library/scheduled-tasks       -> /settings/admin/tasks
/settings/admin/library/scheduled-tasks/queue -> /settings/admin/tasks?tab=queue
/settings/admin/library/scheduled-tasks/activity -> /settings/admin/tasks?tab=activity
/settings/admin/library/scheduled-tasks/flow  -> /settings/admin/tasks?tab=flow
/settings/admin/server/transcoder             -> /settings/admin/transcoding?tab=engine
/settings/admin/server/preroll                -> /settings/admin/transcoding?tab=preroll
/settings/admin/server/notifications          -> /settings/admin/notifications?tab=email
/settings/admin/server/notifications/webhooks -> /settings/admin/notifications?tab=webhooks
/settings/admin/server/credentials            -> /settings/admin/credentials
/settings/admin/server/general                -> /settings/admin/general
/settings/admin/server/network                -> /settings/admin/network
/settings/admin/users/access                  -> /settings/admin/users?tab=access
/settings/admin/users/access-groups           -> /settings/admin/users?tab=groups
/settings/admin/users/devices                 -> /settings/admin/users?tab=devices
/settings/admin/users/invites                 -> /settings/admin/users?tab=invites
/settings/admin/maintenance/backup            -> /settings/admin/maintenance?tab=backups
/settings/admin/maintenance/bulk              -> /settings/admin/maintenance?tab=bulk
/settings/admin/maintenance/logs              -> /settings/admin/logs
/settings/admin/maintenance/logs/audit        -> /settings/admin/logs?tab=audit
```
(The existing `/admin` -> `/settings/admin` redirects stay.) Tab deep-links: server page reads
Next 16 **async** `searchParams` (await it), passes `initialTab` to the client; client mirrors
the active tab back to the URL with `history.replaceState`.

## Components — reuse / extend / build

REUSE: `SettingsCard` + `SettingsRow` (the label/help-left, control-right pattern), `Pill` /
`StatusDot`, `SaveBar`, `Drawer*`, `HeroCard`, `FilterChip`, `ErrorBanner`, `AdminTabBar`, `Pagination`.
BUILD (Phase 1 — **DONE**, in `components/admin/ui/`): `Switch`, `Seg`, `Tabs` (in-page, state-driven),
`StatCard`, `MasterDetail`/`MasterList`/`MasterPane`, `ContextSwitcher`.
BUILD (Phase 2): `SettingsShell` (unified topbar breadcrumb + sidebar + context switch),
`CommandPalette` (⌘K).

## Gating (correction to the synthesized plan)

`/settings/admin/layout.tsx` already calls `requireOwner` — the whole server context is owner-only
**today**. Keep that single gate; do NOT add per-tab owner/admin gating (no current behavior needs it).
Sensitive mutations remain enforced server-side in the Rust API regardless.

## Data fetching

Preserve today's patterns. Dashboard/overview + initial log page: server-fetch for instant render.
Consolidated tab pages: server-fetch the default tab's data, lazy client-fetch secondary tabs on
first open (matches `AdminUsersUnifiedClient`). Wrap SSR awaits in the existing unavailable/error
fallback so one slow section can't blank the page.

## Phases (each left the build green)

- [x] **1. Foundation primitives** — Switch/Seg/Tabs/StatCard/MasterDetail/ContextSwitcher + barrel.
- [x] **2. Unified shell + ⌘K** — SettingsShell wired into `settings/layout.tsx`; admin layout
      defers to it (kept `requireOwner`); `player`→`playback`; CommandPalette.
- [x] **3. Server overview + activity** — `admin/overview` (admin home), `admin/activity` (status/stats);
      Alerts kept standalone & reachable.
- [x] **4. Consolidations A** — `admin/libraries` (5 tabs), `admin/tasks` (4 tabs + kind drill-in),
      `admin/transcoding` (2), `admin/notifications` (2).
- [x] **5. Consolidations B** — `admin/users` (5 tabs, layout removed), `admin/maintenance` (4),
      `admin/logs` (promoted out of maintenance, 2 tabs); You-side `home` + `devices`; Sessions moved
      out of Account into `devices`; `/settings/libraries` split (visibility → `home`, CRUD → Server).
- [x] **6a. Shallow moves** — `server/{general,network,credentials}` flattened to `admin/*`;
      `server/` dir removed.
- [x] **Cleanup** — deleted `SettingsTabs`/`AdminNav`/`AdminMobileNav` + all old route dirs;
      ~30 permanent redirects added; internal `href`s repointed (21 links, 8 files).

**Status: structural migration COMPLETE + polished — `next build` green, lint clean.** Route tree
matches the target IA; every old path 301-redirects.
- [x] **Mobile drawer** — sidebar is desktop-sticky; on small screens a menu button opens a slide-in
      drawer (closes on nav/overlay/Esc). Mobile bar also has a quick ⌘K search button.
- [x] **Switch toggles** — 11 genuine on/off settings swapped from native checkboxes to `<Switch>`
      across 6 forms (general telemetry, transcoder tonemap, preroll enable, 6 library defaults,
      libraries allow-deletion, profile email-notify). Selection checkboxes (bulk-select, access
      matrix) and the already-custom hidden-libraries toggle left as-is.
- [x] **Title consistency** — dropped `AdminPageHeader` on general/network/credentials (credentials
      keeps a one-line encryption note). Kind-detail keeps its dynamic title intentionally.
- [ ] `Seg` not yet swapped into filter bars (FilterChip works; lower priority, mixed single/multi-select).
      `StatCard`/`MasterDetail` primitives built but currently unused — available for future rebuilds.
- [ ] **Browser smoke-test** — type-checked + lint-clean, but NOT yet visually verified in a running
      app (needs Next dev + the Rust backend + an authed session).

## Risks

Split-brain `/settings/libraries` (personal hidden-libs -> `home`; CRUD -> `admin/libraries`).
`player`->`playback` rename: grep `src` for hardcoded `/settings/player` links and fix.
Next 16 async `searchParams`. Owner gate must stay server-side. Tailwind v4 tokens only — no hex.
