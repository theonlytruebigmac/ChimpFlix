# Deployment guide

This is the operator-facing runbook for exposing ChimpFlix to a real
network. It covers reverse-proxy recipes, trusted-proxy configuration,
TLS, and the "before I open port 443" preflight.

The default `docker-compose.yml` is safe **on a trusted LAN with
direct access**. Putting the server behind a reverse proxy on a
public hostname needs the extra steps below.

If you skip this doc, the most common results are:

- Rate limiting and audit logs attributing every request to a single
  Docker bridge IP (because `TRUSTED_PROXIES` is unset).
- The server refusing to start with `vault key required for HTTPS
  deployments` (because `CHIMPFLIX_SECRET_KEY` is unset).
- Player streams hanging mid-segment (because the proxy is timing
  out long-lived HLS or WebSocket requests).

## Preflight checklist

Before exposing the server to the public internet, walk through this
list. Items 1–3 are non-negotiable; the rest are strongly recommended.

1. **Generate and pin a vault key.**
   `openssl rand -hex 32` → set `CHIMPFLIX_SECRET_KEY` in `.env.local`
   and **back it up off-box** (password manager, encrypted note). If
   you ever restore from backup without this key, every encrypted
   secret in the DB is unrecoverable — see
   [docs/PUBLIC_RELEASE_HARDENING.md](PUBLIC_RELEASE_HARDENING.md#1-backupvault-decoupling-silently-bricks-restores).
2. **Set `APP_PUBLIC_ORIGIN`** to the exact `https://host[:port]` users
   will type, no trailing slash. The server uses this for cookies, CSRF
   Origin checks, and HSTS — a mismatch breaks login.
3. **Set `TRUSTED_PROXIES`** to the CIDR your reverse proxy lives in.
   See `.env.example` for the exact lines for "proxy on same Docker
   network" vs "proxy on a different host." If you do not set this,
   per-IP rate limits collapse to a single bucket.
4. **Front the server with TLS.** Caddy and Traefik do this
   automatically; nginx needs a cert (Let's Encrypt via certbot is
   the easiest path).
5. **Block direct access to `:8080`** at the host firewall when the
   proxy lives on a different machine, so an attacker can't bypass
   the proxy by hitting the LAN IP directly.
6. **Complete first-run setup over a private network** (SSH tunnel
   to localhost, or LAN access) *before* DNS points to the box.
   Until BLOCK #5 lands, the `/auth/setup` endpoint has a CSRF-bypass
   window — closing the race by finishing setup before exposure is
   the simplest mitigation.
7. **Pin a `SESSION_SECRET`** if you ever wipe `data/` and want
   active sessions to survive (rare, but needed for blue/green).
8. **Read `docs/PUBLIC_RELEASE_HARDENING.md`** end-to-end so you
   know which items are shipped, which are pending, and what the
   tradeoffs look like.

## Reverse proxy recipes

All three recipes assume:

- ChimpFlix is reachable from the proxy at `chimpflix-server:8080`
  (Docker compose service name) or `192.168.1.20:8080` (different
  host).
- `APP_PUBLIC_ORIGIN=https://flix.example.com` in `.env.local`.

WebSocket support and long-running response support are **required**
— the player streams HLS segments for hours and the admin UI relies
on a persistent `/ws` connection. The recipes below set both.

### Caddy (recommended for new operators)

Caddy auto-provisions TLS, sets sensible forwarded headers, and
proxies WebSockets without extra config. Save as `Caddyfile`:

```caddy
flix.example.com {
    # Auto TLS from Let's Encrypt (default).
    encode zstd gzip

    # Long-running HLS + WebSocket: no idle timeout on the upstream.
    reverse_proxy chimpflix-server:8080 {
        flush_interval -1
        transport http {
            # HLS segment writes can take longer than the default 5m.
            read_timeout    1h
            write_timeout   1h
        }
    }

    # HSTS, X-Frame-Options, X-Content-Type-Options.
    # The server already sends these, but Caddy adds them at the edge too.
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        -Server
    }
}
```

Pair with:

```
TRUSTED_PROXIES=172.16.0.0/12  # if Caddy is in the same docker network
```

### nginx

Save as `/etc/nginx/sites-available/chimpflix`:

```nginx
upstream chimpflix_server {
    server chimpflix-server:8080;
    keepalive 32;
}

server {
    listen 443 ssl http2;
    server_name flix.example.com;

    ssl_certificate     /etc/letsencrypt/live/flix.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/flix.example.com/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;

    # Reasonable headers (the app also sets these; belt + braces).
    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;

    # HLS segments can be 10s+ each and sessions run for hours.
    proxy_read_timeout       1h;
    proxy_send_timeout       1h;
    proxy_buffering          off;
    proxy_request_buffering  off;

    location / {
        proxy_pass         http://chimpflix_server;
        proxy_http_version 1.1;

        # WebSocket upgrade for /ws.
        proxy_set_header Upgrade           $http_upgrade;
        proxy_set_header Connection        "upgrade";

        # Forwarded headers — ChimpFlix uses these when the peer IP
        # is in TRUSTED_PROXIES.
        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}

server {
    listen 80;
    server_name flix.example.com;
    return 301 https://$host$request_uri;
}
```

Pair with:

```
TRUSTED_PROXIES=192.168.1.10/32  # the nginx host
```

### Traefik (Docker labels)

Add to the ChimpFlix server service in `docker-compose.yml`:

```yaml
services:
  chimpflix-server:
    # ... existing config ...
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.chimpflix.rule=Host(`flix.example.com`)"
      - "traefik.http.routers.chimpflix.entrypoints=websecure"
      - "traefik.http.routers.chimpflix.tls.certresolver=le"
      - "traefik.http.services.chimpflix.loadbalancer.server.port=8080"
      # HLS + WS: turn off Traefik's per-connection idle timeout for this router.
      - "traefik.http.routers.chimpflix.middlewares=chimpflix-headers"
      - "traefik.http.middlewares.chimpflix-headers.headers.customrequestheaders.X-Forwarded-Proto=https"
```

In the Traefik static config:

```yaml
entryPoints:
  websecure:
    address: ":443"
    transport:
      respondingTimeouts:
        readTimeout:  1h
        writeTimeout: 1h
        idleTimeout:  1h
    forwardedHeaders:
      # CRITICAL: only honour XFF from your CDN. Otherwise anything
      # reaching Traefik can spoof the client IP. Replace with your
      # Cloudflare / Fastly / Cloudfront ranges.
      trustedIPs:
        - 173.245.48.0/20
        - 103.21.244.0/22
        # ... (Cloudflare ranges, see https://www.cloudflare.com/ips/)
```

Pair with:

```
TRUSTED_PROXIES=172.16.0.0/12  # the Docker bridge Traefik shares with ChimpFlix
```

## Trusted-proxy anti-patterns

These all *look* like they work but silently break rate limiting,
audit logs, or LAN-bypass rules:

- **Leaving `TRUSTED_PROXIES` empty when behind a reverse proxy.**
  All requests appear to come from the proxy's IP, so per-IP rate
  limits become per-proxy (i.e., one bucket for the whole world).
- **Setting `TRUSTED_PROXIES=0.0.0.0/0`.** Trusts every caller —
  including direct internet traffic that bypassed your proxy. An
  attacker just sends `X-Forwarded-For: 1.2.3.4` to spoof any IP.
- **Trusting the Docker bridge (`172.16.0.0/12`) when the proxy is
  on a different host.** The bridge is your *local* container
  network, not the upstream proxy. Wrong CIDR — XFF gets ignored.
- **Configuring `TRUSTED_PROXIES` correctly but leaving `:8080`
  reachable from the LAN.** Attacker connects directly to ChimpFlix
  bypassing the proxy entirely. Block port 8080 at the host firewall
  for any source not in `TRUSTED_PROXIES`.

After changing `TRUSTED_PROXIES`, restart the server and open the
admin home — once WEEK 1 #8 (trusted-proxy diagnostic) ships, the
admin home shows a banner if the detected peer IP looks wrong.

## Other deployment shapes

### Cloudflare Tunnel

You don't need `TRUSTED_PROXIES` for the Tunnel itself, but the
Tunnel daemon (`cloudflared`) acts as your reverse proxy, so the
Caddy-style timeout + WebSocket caveats still apply. In the Cloudflare
dashboard, set the public hostname's "Additional application
settings → Connection → No TLS Verify" only if your origin is HTTP;
prefer running ChimpFlix with a self-signed cert and trusting that.

If you also expose the server to your LAN, set
`TRUSTED_PROXIES=<cloudflared container CIDR>,<lan range>` so both
paths work.

### Tailscale

Tailscale is the simplest "expose to a few friends" path — no public
DNS, no port forwarding, no TLS provisioning. Install Tailscale on the
ChimpFlix host, share the Tailnet with invited users, set
`APP_PUBLIC_ORIGIN=https://chimpflix.<tailnet>.ts.net` and use
Tailscale's MagicDNS + HTTPS cert. `TRUSTED_PROXIES` can stay empty.

### Bare metal (no Docker)

```bash
# Build
cargo build --release -p chimpflix-server

# Run as a systemd unit; see systemd unit template below.
sudo install -m755 target/release/chimpflix-server /usr/local/bin/
```

Minimal `/etc/systemd/system/chimpflix.service`:

```ini
[Unit]
Description=ChimpFlix media server
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
User=chimpflix
Group=chimpflix
WorkingDirectory=/var/lib/chimpflix
EnvironmentFile=/etc/chimpflix.env
ExecStart=/usr/local/bin/chimpflix-server
Restart=on-failure
RestartSec=5
# Resource limits — see BLOCK #3.
MemoryMax=8G
CPUQuota=400%
# Filesystem isolation.
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
NoNewPrivileges=true
ReadWritePaths=/var/lib/chimpflix

[Install]
WantedBy=multi-user.target
```

## Upgrade procedure

1. **Back up first.** `/admin/backups` → "Run backup now" or wait
   for the daily task to fire. Copy the resulting `.db` file off-box,
   along with your `CHIMPFLIX_SECRET_KEY` value (they're a matched
   pair — see preflight #1).
2. **Pull the new image.** `docker compose pull && docker compose up
   -d`. The server runs migrations on boot.
3. **Watch the logs for migration errors.** `docker compose logs -f
   chimpflix-server` for the first minute. If a migration fails, the
   server exits — restore from the backup taken in step 1.
4. **Smoke-check:** open the UI, play one item, run one scan, check
   `/admin/activity` for green status.

Migration rollback is not supported. If a release breaks something
you depend on, restore the pre-upgrade snapshot rather than trying
to "downgrade the schema."

## See also

- [docs/PUBLIC_RELEASE_HARDENING.md](PUBLIC_RELEASE_HARDENING.md) —
  the running hardening plan.
- [docs/ARCHITECTURE.md](ARCHITECTURE.md) — system shape and crate
  boundaries.
- [SECURITY.md](../SECURITY.md) — how to report a vulnerability.
- [.env.example](../.env.example) — every env var the server reads,
  with inline guidance on what to set.
