# V2 Frontend — Web UI Rebuild

> Status: **RFC skeleton.** The largest sub-project in V2.

## Scope

The web UI users see. Browse, modal, search, playback, history, lists,
admin, auth flows, onboarding. Mobile + desktop.

## V1 starting point

- Next.js 16 app router.
- React 19.
- CSS-in-JS via Tailwind utility classes.
- Design-system primitives: Pill, SettingsCard, SaveBar, Drawer,
  HeroCard, FilterChip, AdminTabBar, ConfirmDialog, Pagination,
  useFocusTrap.
- Server-side rendering for some routes (browse, modal).
- SSE for real-time activity.
- Custom video player (ChimpFlixPlayer) using HLS.js or native HLS.

## Open: framework choice

V1's Next.js choice was reasonable. V2 should re-examine because the
rebuild has a different optimization function:

- **Stay on Next.js.** Lowest-friction port. Familiar to maintainer.
  Mature.
- **Astro.** Server-rendered islands. Good for catalog-style pages,
  cheaper hydration. Less great for highly-interactive surfaces (the
  player, modal, drag-and-drop list editing).
- **SolidStart.** Solid's reactivity primitives are excellent for the
  player UI. SSR + islands. Newer ecosystem.
- **Remix.** Now part of React Router. Strong data-loading story.
  Less polish than Next.js for our specific patterns.
- **Stick with Next.js but redesign the architecture.** Drop ambient
  fetching patterns, lean harder on Server Components for catalog,
  simplify client-side state.

Recommendation: **stick with Next.js, redesign the architecture.**
Framework migrations are expensive and don't address V1's actual
frontend pains (state management complexity, hydration footprint,
ambient `fetch` patterns). The opportunity is to get the
architecture clean inside the same framework, not switch.

But this is a real conversation, not a decided call. Decide in
Phase 0 alongside this RFC.

## Carry forward from V1

- Netflix-style home with hero + rails.
- Title modal with Up Next, More Like This, file info, cast.
- Browse by library with filter chips + sorts + pagination.
- Search with bm25-ranked results.
- Player with hotkeys, chapter menu, skip-intro UI, mobile scrubber,
  resume pill.
- Onboarding wizard.
- Admin surfaces using the design-system primitives.
- Accessibility: focus traps, arrow nav, aria-live, focus rings.
- Trakt + Plex OAuth flows.
- The visual + interaction spec V1 has settled into is V2's target.
  This is not a UX redesign.

## What changes

- **State management discipline.** V1 has accumulated context
  providers, event buses (ratings cross-component bus), and useEffect
  cascades. V2 leans on URL state for navigation-level things,
  TanStack Query (or equivalent) for server state, and minimal
  client state. The cross-component event bus pattern is a smell V2
  doesn't carry.
- **Type-safe API client.** V2's API choice (`API.md`) determines
  the client shape. End-to-end type safety from server → client is
  a hard requirement.
- **CSP without `unsafe-eval`.** V1's CSP needs `unsafe-eval` for
  Next 16 dev + React 19 hydration. V2 targets a production CSP
  without it (production-build-only, dev keeps the unsafe-eval since
  it's required by the stack). Document the constraint.
- **Server Component lean.** Where V1 has client components fetching
  data on mount, V2 uses Server Components to fetch + render
  static-ish surfaces, with islands for interactivity. Reduces
  hydration cost on browse-heavy pages.
- **Image handling.** V1 uses Next/Image with onError fallbacks.
  V2 evaluates whether the network fetch + sharp transform is worth
  it or if a simpler `<img>` with a thumbnail service is enough.

## Open questions

- **Framework.** Recommendation above; confirm or change.
- **TanStack Query vs. Server Components.** Heavily Server Component
  pages reduce the need for a client cache. Hybrid approach likely;
  draw the line in Phase 5.
- **Player.** ChimpFlixPlayer is a custom React wrapper around
  hls.js. Worth re-evaluating: HTML5 media element + a thin React
  controls layer might be simpler.
- **Mobile fullscreen on iOS.** V1 uses `webkitbeginfullscreen` sync.
  V2 keeps this until Safari ships standards-compliant Fullscreen
  API.
- **Casting.** V1 has device detection + improved UI. V2 evaluates
  Cast SDK integration vs. native picker. Probably ports V1 unchanged
  for the first cut.

## Cut list

- **Cross-component event buses.** Replaced by Query invalidation +
  Server Component re-rendering.
- **Bespoke icon set.** Use a maintained library (Lucide, Tabler).
  V1 mostly does this; V2 commits to it fully.
- **Server-side TMDB-side rendering for SEO.** ChimpFlix is private.
  No SEO surface.
