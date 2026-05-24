# Security policy

ChimpFlix is a self-hosted media server. The threat model assumes a
public reverse-proxy in front of the server, invite-only registration,
and operators who range from skilled to "first time touching Docker."
This document explains how to report security issues responsibly and
what guarantees the project makes back.

## Reporting a vulnerability

**Do not open a public GitHub issue for security problems.** Use one of
the following private channels instead:

- **Preferred:** GitHub's [private security advisories](https://github.com/soybigmac/ChimpFlix/security/advisories/new)
  on this repository. Anyone with a GitHub account can open one — they
  are visible only to the maintainers and to people you invite onto
  the report.
- If GitHub is unreachable or you cannot use a GitHub account, open a
  GitHub issue containing only "I would like a private channel to
  report a security issue, please reach out" and we will arrange one.
  Do **not** include details in the public issue.

A good report includes:

- ChimpFlix version (commit SHA if running from `main`).
- Deployment shape (Docker / bare-metal, reverse proxy in front,
  trusted-proxy CIDRs configured).
- Reproduction steps or proof-of-concept.
- Impact assessment from your perspective (what an attacker can do).
- Whether the issue is already public anywhere.

## Response SLA

- **Acknowledgement:** within 7 days of report.
- **Triage + severity assessment:** within 14 days.
- **Fix target:**
  - Critical (RCE, auth bypass, data loss): 30 days.
  - High (privilege escalation, sensitive disclosure): 60 days.
  - Medium / low: best-effort, batched into the next release.
- **Coordinated disclosure:** by default, we hold the advisory private
  until a fix is released, then publish under
  `https://github.com/soybigmac/ChimpFlix/security/advisories`. If you
  want a different timeline (e.g. an academic publication date), say
  so in the report and we will negotiate.

## Supported versions

ChimpFlix is **pre-1.0** active development. Until v1.0:

| Version       | Security fixes |
| ------------- | -------------- |
| `main` branch | Yes            |
| Latest `0.x`  | Yes            |
| Older `0.x`   | No             |

After v1.0 ships, this table will be replaced with a more conservative
support window.

## What the server does NOT do

These are commitments — if you find any of them violated, treat it as a
security report:

- **No telemetry / no phone-home.** The server makes outbound HTTP
  calls only to services the operator explicitly configures (TMDB,
  AniList, Trakt, OpenSubtitles, and SMTP). No analytics. No
  auto-update check. No crash reporting upstream.
- **No data leaves the box except for those configured upstreams.**
  Backup files stay on disk where the operator put them; user metadata
  is never synced anywhere by default.
- **No third-party JavaScript in the web frontend.** The Next.js build
  is fully self-hosted; no Google fonts, no analytics tags, no CDN
  scripts.

## Out of scope

- Self-XSS or attacks requiring the victim to paste attacker-supplied
  content into the browser devtools.
- DDoS / volumetric attacks against the network layer — operators are
  expected to front the server with a CDN, reverse proxy, or
  rate-limiting infrastructure of their choice.
- Social engineering against operators or invited users.
- Vulnerabilities in third-party services the operator configures
  (TMDB, AniList, etc.) — report those to the relevant service.
- Brute-forcing valid usernames via response timing — the auth path
  uses constant-time comparisons but cannot prevent network-level
  inference.

## Bug bounty

ChimpFlix is a volunteer project and does not currently pay bounties.
Credit in the advisory + an entry in `CHANGELOG.md` is offered for
valid reports, with the reporter's preferred name and link (or
anonymous if requested).

## See also

- [docs/PUBLIC_RELEASE_HARDENING.md](docs/PUBLIC_RELEASE_HARDENING.md)
  — the project's running hardening plan, including "confirmed fine"
  invariants the audit looked at and accepted.
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) — reverse-proxy + TLS +
  trusted-proxy guidance for safe public exposure.
