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
    // come from i.ytimg.com. The metadata enrichment pipeline can also
    // hit TVMaze (fallback show provider), TheTVDB (anime / fallback),
    // and AniList (anime primary) — their image CDNs need to be in
    // img-src or browsers block the poster/backdrop loads.
    "img-src 'self' data: blob: https://i.ytimg.com https://image.tmdb.org https://static.tvmaze.com https://artworks.thetvdb.com https://s4.anilist.co",
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
    return [
      {
        source: "/:path*",
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
    ];
  },
};

export default nextConfig;
