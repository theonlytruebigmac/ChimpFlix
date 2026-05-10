# ChimpFlix

A Netflix-style frontend for a personal Plex Media Server. Plex still does the
heavy lifting — library metadata, transcoding, HLS streaming, watch state.
ChimpFlix is a fast Next.js app that re-skins the browse and playback experience
in the Netflix visual language.

> Not affiliated with Netflix or Plex. Built for personal homelab use against a
> Plex server the operator already owns or has been granted access to.

## Features

- **Sign in with Plex** — OAuth redirect (no manual PIN entry), supports the
  account owner plus any Plex Home managed users.
- **Multi-server / multi-tenant** — anyone with access to a Plex server you
  share gets a picker for every server their plex.tv account can see.
- **Netflix-style browse** — hero rotation, curated rails on `/`, `/movies`,
  `/shows`, `/new-popular`, hover-preview cards, modal "more info", "My List",
  search, genre pages.
- **Player parity** — HLS via hls.js, transcoder session reuse, watch-state
  sync (`:/timeline`), PiP, episodes panel, keyboard shortcuts, time-remaining,
  skip intro/credits markers from Plex.
- **TMDB trailer autoplay** — modal and home hero swap to a muted YouTube
  embed after a short delay when a trailer is available.
- **Background warmer + service worker** — first paint is cached HTML; the
  warmer prefetches sections / on-deck / recently-added in the background so
  navigation feels instant.

## Quick start (local dev)

```bash
cp .env.example .env.local
# Fill in PLEX_CLIENT_IDENTIFIER and SESSION_SECRET — generation
# commands are in .env.example.

npm install
npm run dev
# open http://localhost:3000
```

You'll be redirected to `/login` on the first hit. After signing in with Plex,
pick a server, and you land on the home page.

## Docker

The included `Dockerfile` produces a standalone Next.js image that runs as a
non-root user (uid 1000). `docker-compose.yml` is a single-service example.

```bash
# Create the state directory and make it writable by the container's uid 1000.
mkdir -p ./.app-state
sudo chown -R 1000:1000 ./.app-state

# Configure env (use the same .env.local as `npm run dev`, or create a
# fresh one on the deploy host).
cp .env.example .env.local
# ...edit values...

docker compose up -d --build
# http://localhost:3000
```

### Hosting alongside Plex in Docker

If your Plex server runs in Docker bridge mode on the same host, Plex
advertises its container bridge IP as the "local" connection — which other
containers can't reach. Two ways to fix it:

**Option A** — add ChimpFlix to Plex's docker network:

```bash
docker network ls | grep plex   # find Plex's network name
```

Uncomment the `networks:` block at the bottom of [docker-compose.yml](docker-compose.yml)
and replace `plex_default` with the name you found. ChimpFlix can then reach
Plex on its bridge IP at LAN speed.

**Option B** — add a custom Plex URL. In Plex Web → Settings → Network →
"Custom server access URLs", add `http://YOUR_HOST_LAN_IP:32400`. Plex starts
advertising that address too, which works from anywhere on your LAN.

### Production (HTTPS, reverse proxy)

Set `APP_PUBLIC_ORIGIN=https://your.domain` in `.env.local` so:

- Cookies are flagged `Secure`.
- Plex OAuth's `forwardUrl` is built from the trusted origin, not from
  attacker-controllable `Host` headers.
- The CSRF middleware compares request `Origin` against the canonical value.

If you terminate TLS at a reverse proxy (Traefik, nginx, Caddy) and want
`X-Forwarded-Proto` honored even without `APP_PUBLIC_ORIGIN`, set
`APP_TRUST_PROXY=1`.

## Configuration

All env vars live in `.env.local`. See [.env.example](.env.example) for
generation commands and defaults.

| Variable | Required | Purpose |
| --- | --- | --- |
| `PLEX_CLIENT_IDENTIFIER` | yes | Stable UUID this app presents to plex.tv. Generate once and keep. |
| `SESSION_SECRET` | yes | HMAC secret used to sign cookies. 32+ random bytes. |
| `NEXT_PUBLIC_BRAND_NAME` | no | UI wordmark and page titles. Defaults to `ChimpFlix`. Rebuild required after change. |
| `NEXT_PUBLIC_BRAND_NAME_UPPER` | no | Override for the all-caps wordmark. Defaults to the brand name uppercased. |
| `TMDB_READ_TOKEN` | no | TMDB v4 read token. Enables trailer autoplay. |
| `PLEX_PRODUCT_NAME` | no | Device label shown in plex.tv's authorized devices. Defaults to `ChimpFlix`. |
| `PLEX_DEVICE_NAME` | no | Same, for the device-name field. |
| `PLEX_SERVER_URL` | no | Legacy single-server fallback. Leave blank for new deployments. |
| `APP_PUBLIC_ORIGIN` | no | Canonical user-facing origin (scheme + host). Use in any Internet-facing deployment. |
| `APP_TRUST_PROXY` | no | `1` to honor `X-Forwarded-Proto` from a reverse proxy. |
| `APP_STATE_DIR` | no | Directory where the bootstrap auth file is persisted. Defaults to the working dir. |

### Rebrand

Set `NEXT_PUBLIC_BRAND_NAME=YourName` in `.env.local` and rebuild. The
wordmark in the nav, the login screen, the modal "Original" tag, the
browser tab title, and every "New on …" rail will all switch over. The
internal cookie / env / state-file prefixes (`cf_*`, `APP_*`,
`.app-state/`) are brand-neutral and don't need to change.

## Architecture

```text
Browser ──── ChimpFlix (Next.js server) ──── Plex Media Server
                │
                └── plex.tv (OAuth + resource discovery)
```

- **Auth**: OAuth-style PIN flow (`strong=true`) redirects through plex.tv,
  comes back with a token written into an httpOnly signed cookie. A separate
  per-server access token is stored alongside, scoped by `clientIdentifier`.
- **Proxy** (`/api/plex/[...path]`): browser never sees the Plex token. The
  proxy attaches it server-side and streams the response back. State-changing
  Plex GETs (`:/timeline`, `:/scrobble`, etc.) require a matching `Origin`
  header so cross-origin `<img>` tags can't forge watch markers.
- **Cache warmer**: at boot, the persisted bootstrap auth fires off sections /
  on-deck / recently-added fetches in the background so the first user
  navigation lands on warm data.
- **Service worker**: stale-while-revalidate for navigation HTML, cache-first
  for static assets and images.

### Source layout

```text
src/
  app/
    api/
      auth/         # OAuth + session + server picker
      plex/         # token-injecting upstream proxy
      modal/        # title-modal data endpoint
      prefs/        # hidden-libraries cookie
      tmdb/         # trailer lookup
    login/          # OAuth sign-in screen
    select-server/  # Plex server picker (post sign-in)
    movies/ shows/ new-popular/ search/ my-list/ genre/[name]/
    watch/[ratingKey]/
    page.tsx        # home
  components/       # client UI
  lib/              # env, plex client, session, cache, warmer
  middleware.ts     # CSRF origin check for state-changing API routes
```

## Security

The auth and proxy layers have been pass-audited end to end. Highlights:

- httpOnly signed cookies for all tokens. `Secure` flag driven by
  `APP_PUBLIC_ORIGIN` (authoritative) or `APP_TRUST_PROXY` (proxy-honored), not
  by trusting raw `X-Forwarded-Proto`.
- CSRF origin check via middleware on `/api/auth/*`, `/api/prefs/*`, and
  `/api/plex/*` (non-safe methods). State-changing Plex GETs additionally
  enforce an Origin check inside the proxy, with path normalization to defeat
  encoded/case/traversal bypasses.
- SSRF defense on the connection picker: scheme allowlist, literal block-list
  (loopback, link-local, IPv4-mapped IPv6, numeric IPv4), and a DNS-resolution
  check before any URL is committed to the session cookie.
- Plex API errors are scrubbed of `authToken` / `accessToken` / `X-Plex-Token`
  before being surfaced to the browser. Full bodies log server-side only.
- Container runs as `node` (uid 1000), not root.

## Development

```bash
npm run dev      # turbopack dev server
npm run build    # production build (standalone output for Docker)
npm run lint     # eslint
```

The cache warmer is gated on `process.env.NODE_ENV === "production"` so dev
hot-reload doesn't trigger it. To exercise the warmer locally, run
`npm run build && npm start`.

## Tech stack

- [Next.js 16](https://nextjs.org/) (App Router, Turbopack, standalone output)
- [React 19](https://react.dev/)
- [Tailwind CSS 4](https://tailwindcss.com/)
- [hls.js](https://github.com/video-dev/hls.js) for HLS playback in browsers
  that don't natively support it.

## License

This project ships under no license by default — it's a personal homelab tool.
Add a `LICENSE` file before sharing if that matters for your use.
