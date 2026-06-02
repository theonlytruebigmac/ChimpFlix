import type { NextConfig } from "next";

const apiTarget = process.env.CHIMPFLIX_API_URL ?? "http://localhost:8080";
const publicOrigin = process.env.APP_PUBLIC_ORIGIN;

/**
 * Content-Security-Policy for HTML responses. The app loads YouTube
 * trailer embeds (youtube-nocookie.com) and HLS over /api/v1/stream/…,
 * but otherwise stays first-party.
 *
 * `script-src 'self' 'unsafe-inline'` is unfortunately required by
 * Next.js's app-router runtime (per-page inline bootstrap chunks).
 * Tighten to nonce-based when Next ships first-class CSP nonce support.
 */
function csp(): string {
  return [
    "default-src 'self'",
    "base-uri 'self'",
    "form-action 'self'",
    // YouTube iframe for trailers + same-origin for fullscreen player.
    "frame-src https://www.youtube-nocookie.com https://www.youtube.com",
    "frame-ancestors 'none'",
    // Inline styles + scripts: Next App Router emits inline bootstrap.
    // `'unsafe-eval'` is still required: Next.js's dev mode honors
    // these headers (contrary to a prior assumption) and uses eval for
    // fast refresh; React 19 Server Component hydration also relies on
    // Function() in some paths. Round-3 tried to drop it and broke
    // client hydration in dev (Suspense fallbacks never resolved).
    // Re-evaluate when we move to nonce-based CSP.
    //
    // `https://www.gstatic.com` hosts the Google Cast Web Sender SDK
    // (`cast_sender.js` + the chained `cast_framework.js`). Without it
    // here the SDK never loads on Android Chrome / installed PWAs — CSP
    // blocks the script tag and the cast button stays hidden because
    // `__onGCastApiAvailable` never fires. Desktop Chrome sometimes
    // worked anyway because the Cast bridge can be wired through a
    // CSP-exempt browser extension on that platform, but the PWA path
    // is strictly CSP-gated.
    "script-src 'self' 'unsafe-inline' 'unsafe-eval' https://www.gstatic.com",
    "style-src 'self' 'unsafe-inline'",
    // Posters / backdrops served from same origin; trailer thumbnails
    // come from i.ytimg.com; metadata-CDN hosts (TMDB, TVMaze, TheTVDB,
    // AniList) supply enrichment art. `https:` is included so user-set
    // **avatar URLs** (Account → Profile accepts "a direct image URL")
    // and any other external image load instead of being CSP-blocked —
    // a deliberate relaxation for a self-hosted app where users supply
    // their own URLs. (Note: some hosts like LinkedIn hotlink-protect and
    // may still 403 regardless of CSP; a direct image host works best.)
    "img-src 'self' data: blob: https:",
    "font-src 'self' data:",
    // Plex's CDN for show theme songs played on the title modal.
    // Same source the official Plex web app uses; first-party MP3.
    "media-src 'self' blob: https://tvthemes.plexapp.com",
    "connect-src 'self'",
    "object-src 'none'",
    "worker-src 'self' blob:",
    "manifest-src 'self'",
  ].join("; ");
}

/**
 * CSP for the Google Cast receiver page (/cast/receiver.html). The
 * receiver runs on the Cast device, not in a normal browser, and has
 * needs the main-app CSP doesn't:
 *
 *  - `script-src https://www.gstatic.com` loads the CAF receiver
 *    framework (same host the sender SDK uses).
 *  - `connect-src ws: wss:` — the CAF runtime talks to the Cast
 *    platform over a local WebSocket control channel; `connect-src
 *    'self'` would block it on devices that route that channel through
 *    page CSP, and the receiver would never start.
 *  - HLS manifests/segments are fetched same-origin (`'self'`); MSE
 *    feeds the decoder via `blob:`; posters come from the same CDNs the
 *    app uses.
 *
 * This is served ONLY for /cast/* (the global matcher below excludes
 * that prefix) so the looser `ws:`/`wss:` connect-src never widens the
 * policy on the main application surface.
 */
function castReceiverCsp(): string {
  return [
    "default-src 'self'",
    "base-uri 'self'",
    "frame-ancestors 'none'",
    "script-src 'self' 'unsafe-inline' 'unsafe-eval' https://www.gstatic.com",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob: https://image.tmdb.org https://static.tvmaze.com https://artworks.thetvdb.com https://s4.anilist.co",
    "media-src 'self' blob:",
    "connect-src 'self' https://www.gstatic.com ws: wss:",
    "font-src 'self' data:",
    "object-src 'none'",
  ].join("; ");
}

const securityHeaders = [
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  {
    // Permissions-Policy: keep destructive surfaces (camera, mic,
    // geolocation, payment, USB) hard-blocked even from this origin,
    // but allow benign sensors that the player and any third-party
    // library may legitimately want to read. Locking sensors to `()`
    // (no origin allowed) caused a tight React render loop on Android
    // Chrome PWAs: a bundled dependency tried to subscribe to
    // accelerometer / deviceorientation each render, hit the violation
    // synchronously, never finished the effect, and React kept
    // rescheduling forever — symptom was a black video with
    // unresponsive controls. Allowing `self` lets the subscription
    // resolve (with or without real data) so the player can mount.
    //
    // YouTube's embedded player (used for hero trailers) also calls
    // accelerometer / gyroscope / deviceorientation synchronously on
    // load. The iframe's `allow=` attribute only delegates features
    // the *parent* already holds, so we whitelist youtube-nocookie.com
    // alongside `self`. Without that origin in the list every hero
    // trailer mount logs ~70 violation entries plus a render-loop
    // burst that visibly slows the home page.
    key: "Permissions-Policy",
    value: [
      'accelerometer=(self "https://www.youtube-nocookie.com")',
      'gyroscope=(self "https://www.youtube-nocookie.com")',
      'magnetometer=(self "https://www.youtube-nocookie.com")',
      'fullscreen=(self "https://www.youtube-nocookie.com")',
      'picture-in-picture=(self "https://www.youtube-nocookie.com")',
      "screen-wake-lock=(self)",
      'autoplay=(self "https://www.youtube-nocookie.com")',
      'encrypted-media=(self "https://www.youtube-nocookie.com")',
      "camera=()",
      "geolocation=()",
      "microphone=()",
      "payment=()",
      "usb=()",
    ].join(", "),
  },
  { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
  // Frontend serves cross-origin resources via /api/v1/* (images, HLS
  // segments). same-site lets browsers fetch those even when the page
  // is loaded under a different subdomain in the deployment.
  { key: "Cross-Origin-Resource-Policy", value: "same-site" },
  { key: "Content-Security-Policy", value: csp() },
  ...(publicOrigin?.startsWith("https://")
    ? [
        {
          key: "Strict-Transport-Security",
          value: "max-age=31536000; includeSubDomains",
        },
      ]
    : []),
];

const nextConfig: NextConfig = {
  output: "standalone",
  allowedDevOrigins: ["172.16.103.228"],
  // Hide framework identity from probes. Defense-in-depth — knowing the
  // exact Next.js version is information an attacker doesn't need.
  poweredByHeader: false,
  async headers() {
    // The Cast receiver gets the same hardening as the rest of the app
    // (nosniff, frame-deny, COOP/CORP, HSTS) but a CSP tuned for the CAF
    // runtime — see `castReceiverCsp`. Multiple matching CSP headers are
    // enforced as an intersection (most restrictive wins), so the global
    // matcher below MUST exclude /cast/ or the receiver would inherit the
    // app's `connect-src 'self'` and the Cast control channel would break.
    const castReceiverHeaders = securityHeaders.map((h) =>
      h.key === "Content-Security-Policy"
        ? { key: h.key, value: castReceiverCsp() }
        : h,
    );
    return [
      {
        source: "/cast/:path*",
        headers: castReceiverHeaders,
      },
      {
        // Everything except /cast/* (carved out above). Next.js supports
        // a negative-lookahead in the path so the receiver isn't also
        // served the app CSP, which would intersect and re-block ws:/wss:.
        source: "/((?!cast/).*)",
        headers: securityHeaders,
      },
    ];
  },
  async rewrites() {
    // Proxy /api/v1/* to the Rust backend so the browser sees the whole
    // app on a single origin. The legacy /api/auth/* Plex routes are
    // Next.js route handlers in src/app/api and are left untouched.
    return [
      {
        source: "/api/v1/:path*",
        destination: `${apiTarget}/api/v1/:path*`,
      },
    ];
  },
  async redirects() {
    // Phase 16 moved the admin console under /settings/admin/* so the
    // unified Settings shell can host both the user tabs and the admin
    // tabs side by side. Keep the old /admin/* paths working — bookmarks,
    // audit-log entries, copy-pasted URLs all still resolve.
    return [
      { source: "/admin", destination: "/settings/admin", permanent: true },
      {
        source: "/admin/:path*",
        destination: "/settings/admin/:path*",
        permanent: true,
      },
      // Settings/admin IA redesign (branch redesign/settings-admin-ia).
      // Old paths keep resolving as routes consolidate. More mappings are
      // added here as each phase lands its new route.
      {
        source: "/settings/player",
        destination: "/settings/playback",
        permanent: true,
      },
      // Phase 3: admin landing + stats promoted out of the deep tree.
      {
        source: "/settings/admin",
        destination: "/settings/admin/overview",
        permanent: true,
      },
      {
        source: "/settings/admin/status/stats",
        destination: "/settings/admin/activity",
        permanent: true,
      },
      // Phase 4: server/transcoder + preroll -> transcoding (tabs);
      // server/notifications + webhooks -> notifications (tabs).
      {
        source: "/settings/admin/server/transcoder",
        destination: "/settings/admin/transcoding?tab=engine",
        permanent: true,
      },
      {
        source: "/settings/admin/server/preroll",
        destination: "/settings/admin/transcoding?tab=preroll",
        permanent: true,
      },
      {
        source: "/settings/admin/server/notifications",
        destination: "/settings/admin/notifications?tab=email",
        permanent: true,
      },
      {
        source: "/settings/admin/server/notifications/webhooks",
        destination: "/settings/admin/notifications?tab=webhooks",
        permanent: true,
      },
      // Phase 4b: library subtree -> single tabbed /libraries page.
      {
        source: "/settings/admin/library",
        destination: "/settings/admin/libraries?tab=defaults",
        permanent: true,
      },
      {
        source: "/settings/admin/library/libraries",
        destination: "/settings/admin/libraries?tab=libraries",
        permanent: true,
      },
      {
        source: "/settings/admin/library/collections",
        destination: "/settings/admin/libraries?tab=collections",
        permanent: true,
      },
      {
        source: "/settings/admin/library/agents",
        destination: "/settings/admin/libraries?tab=agents",
        permanent: true,
      },
      {
        source: "/settings/admin/library/versions",
        destination: "/settings/admin/libraries?tab=optimized",
        permanent: true,
      },
      // Phase 4b: scheduled-tasks subtree -> single tabbed /tasks page
      // (per-kind drill-in moves under /tasks/kind).
      {
        source: "/settings/admin/library/scheduled-tasks",
        destination: "/settings/admin/tasks",
        permanent: true,
      },
      {
        source: "/settings/admin/library/scheduled-tasks/queue",
        destination: "/settings/admin/tasks?tab=queue",
        permanent: true,
      },
      {
        source: "/settings/admin/library/scheduled-tasks/activity",
        destination: "/settings/admin/tasks?tab=activity",
        permanent: true,
      },
      {
        source: "/settings/admin/library/scheduled-tasks/flow",
        destination: "/settings/admin/tasks?tab=flow",
        permanent: true,
      },
      {
        source: "/settings/admin/library/scheduled-tasks/kind/:kind*",
        destination: "/settings/admin/tasks/kind/:kind*",
        permanent: true,
      },
      // Phase 5: users sub-pages -> tabs on /users (base path unchanged).
      {
        source: "/settings/admin/users/access",
        destination: "/settings/admin/users?tab=access",
        permanent: true,
      },
      {
        source: "/settings/admin/users/access-groups",
        destination: "/settings/admin/users?tab=groups",
        permanent: true,
      },
      {
        source: "/settings/admin/users/devices",
        destination: "/settings/admin/users?tab=devices",
        permanent: true,
      },
      {
        source: "/settings/admin/users/invites",
        destination: "/settings/admin/users?tab=invites",
        permanent: true,
      },
      // Phase 5: maintenance backup/bulk -> tabs; logs promoted to /logs.
      {
        source: "/settings/admin/maintenance/backup",
        destination: "/settings/admin/maintenance?tab=backups",
        permanent: true,
      },
      {
        source: "/settings/admin/maintenance/bulk",
        destination: "/settings/admin/maintenance?tab=bulk",
        permanent: true,
      },
      {
        source: "/settings/admin/maintenance/logs",
        destination: "/settings/admin/logs",
        permanent: true,
      },
      {
        source: "/settings/admin/maintenance/logs/audit",
        destination: "/settings/admin/logs?tab=audit",
        permanent: true,
      },
      // Phase 5/6: flatten server/{general,network,credentials} up a level;
      // personal libraries page splits into home (visibility) + the
      // owner-only Server -> Libraries CRUD.
      {
        source: "/settings/admin/server/general",
        destination: "/settings/admin/general",
        permanent: true,
      },
      {
        source: "/settings/admin/server/network",
        destination: "/settings/admin/network",
        permanent: true,
      },
      {
        source: "/settings/admin/server/credentials",
        destination: "/settings/admin/credentials",
        permanent: true,
      },
      {
        source: "/settings/libraries",
        destination: "/settings/home",
        permanent: true,
      },
    ];
  },
};

export default nextConfig;
