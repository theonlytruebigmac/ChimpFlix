"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  type LastReachability,
  type NetworkSettings,
  type ReachabilityResult,
  type SecureConnectionsMode,
} from "@/lib/chimpflix-api";

export function AdminNetworkClient({ initial }: { initial: NetworkSettings }) {
  // See AdminGeneralForm for rationale. Track the dirty-check
  // baseline in local state so save success doesn't have to mutate
  // the `initial` prop in place.
  // Defensive `?? ""` on every string field — earlier the backend
  // NetworkResponse forgot to include `bind_interface`, the field
  // arrived as `undefined`, and the next `bindInterface.trim()` on
  // save threw "cannot access property 'trim' of undefined" — which
  // surfaced as "Save failed: can't access property trim, k is
  // undefined" in the UI when adjusting the CORS allowlist. Backend
  // now ships the field, but defaulting to "" here means any future
  // schema drift can't crash this form again.
  const [baseline, setBaseline] = useState({
    public_url: initial.public_url ?? null,
    cors_origins: initial.cors_origins ?? [],
    secure_connections: initial.secure_connections,
    transcoder_reaper_idle_threshold_ms:
      initial.transcoder_reaper_idle_threshold_ms,
    max_remote_streams_per_user: initial.max_remote_streams_per_user,
    lan_networks: initial.lan_networks ?? "",
    auth_bypass_cidrs: initial.auth_bypass_cidrs ?? "",
    bind_interface: initial.bind_interface ?? "",
  });
  const [publicUrl, setPublicUrl] = useState(baseline.public_url ?? "");
  const [origins, setOrigins] = useState(baseline.cors_origins.join("\n"));
  const [secure, setSecure] = useState<SecureConnectionsMode>(
    baseline.secure_connections,
  );
  const [reaperMs, setReaperMs] = useState(
    baseline.transcoder_reaper_idle_threshold_ms,
  );
  const [remoteCap, setRemoteCap] = useState(baseline.max_remote_streams_per_user);
  const [lanNetworks, setLanNetworks] = useState(baseline.lan_networks);
  const [bypassCidrs, setBypassCidrs] = useState(baseline.auth_bypass_cidrs);
  const [bindInterface, setBindInterface] = useState(baseline.bind_interface);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [check, setCheck] = useState<ReachabilityResult | null>(null);
  const [checking, setChecking] = useState(false);
  // Standing "last checked" snapshot. Seeded from the persisted value the
  // backend stores on every reachability run, then refreshed in place when
  // the operator runs a manual check so the "checked Xm ago" label is
  // honest immediately (without waiting for a page reload).
  const [lastReach, setLastReach] = useState<LastReachability | null>(
    initial.last_reachability ?? null,
  );
  // 2.5s "Saved." flash after a successful save; cleared on unmount so
  // a late timer can't setState against a torn-down node.
  const flashTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (flashTimerRef.current !== null) {
        window.clearTimeout(flashTimerRef.current);
      }
    };
  }, []);

  const originsParsed = origins
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

  const dirtyFields: Record<string, boolean> = {
    "Public URL": (publicUrl || null) !== baseline.public_url,
    "CORS allowlist":
      JSON.stringify(originsParsed) !== JSON.stringify(baseline.cors_origins),
    "Secure connections": secure !== baseline.secure_connections,
    "Reap idle sessions":
      reaperMs !== baseline.transcoder_reaper_idle_threshold_ms,
    "Max remote streams": remoteCap !== baseline.max_remote_streams_per_user,
    "LAN networks": lanNetworks !== baseline.lan_networks,
    "Allow without auth": bypassCidrs !== baseline.auth_bypass_cidrs,
    "Bind interface": bindInterface !== baseline.bind_interface,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, isDirty]) => isDirty)
    .map(([label]) => label);
  const dirtyCount = dirtyLabels.length;
  const summary =
    dirtyLabels.slice(0, 3).join(", ") +
    (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "");

  // The reaper threshold and bind_interface are consumed at process
  // start and not hot-reloaded. Flag visibly so the operator knows why
  // their change doesn't kick in until restart.
  const reaperChanged = dirtyFields["Reap idle sessions"];
  const bindChanged = dirtyFields["Bind interface"];

  async function save() {
    if (busy || dirtyCount === 0) return;
    // Guard numeric fields — Number('') is 0, which bypasses the HTML min
    // attribute (advisory only on type="number" with onClick-driven saves).
    if (!Number.isFinite(reaperMs) || reaperMs < 5000) {
      setError("Reaper threshold must be at least 5000 ms");
      return;
    }
    if (!Number.isFinite(remoteCap) || remoteCap < 0) {
      setError("Max remote streams must be 0 or greater");
      return;
    }
    setBusy(true);
    setError(null);
    setSaved(false);
    const patch = {
      public_url: publicUrl.trim() || null,
      cors_origins: originsParsed,
      secure_connections: secure,
      transcoder_reaper_idle_threshold_ms: reaperMs,
      max_remote_streams_per_user: remoteCap,
      lan_networks: lanNetworks.trim(),
      auth_bypass_cidrs: bypassCidrs.trim(),
      bind_interface: bindInterface.trim(),
    };
    try {
      await adminApi.network.patch(patch);
      setBaseline({
        public_url: patch.public_url,
        cors_origins: patch.cors_origins,
        secure_connections: patch.secure_connections,
        transcoder_reaper_idle_threshold_ms:
          patch.transcoder_reaper_idle_threshold_ms,
        max_remote_streams_per_user: patch.max_remote_streams_per_user,
        lan_networks: patch.lan_networks,
        auth_bypass_cidrs: patch.auth_bypass_cidrs,
        bind_interface: patch.bind_interface,
      });
      setSaved(true);
      if (flashTimerRef.current !== null) {
        window.clearTimeout(flashTimerRef.current);
      }
      flashTimerRef.current = window.setTimeout(() => {
        flashTimerRef.current = null;
        setSaved(false);
      }, 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  function discard() {
    setPublicUrl(baseline.public_url ?? "");
    setOrigins(baseline.cors_origins.join("\n"));
    setSecure(baseline.secure_connections);
    setReaperMs(baseline.transcoder_reaper_idle_threshold_ms);
    setRemoteCap(baseline.max_remote_streams_per_user);
    setLanNetworks(baseline.lan_networks);
    setBypassCidrs(baseline.auth_bypass_cidrs);
    setBindInterface(baseline.bind_interface);
  }

  async function runCheck() {
    setChecking(true);
    setError(null);
    try {
      const r = await adminApi.network.testReachability();
      setCheck(r);
      // The endpoint persisted this same result server-side; mirror it into
      // the standing banner immediately so "checked Xm ago" stays honest
      // without a reload. checked_at is now (the backend stamps the same).
      setLastReach({
        ok: r.ok,
        public_url: r.public_url,
        status_code: r.status_code,
        latency_ms: r.latency_ms,
        error: r.error,
        checked_at: Date.now(),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setChecking(false);
    }
  }

  const diag = initial.proxy_diagnostic;

  return (
    <div>
      {/* ── top action: on-demand reachability check ─────────────────── */}
      <div
        className="cf-flex cf-between"
        style={{ marginBottom: 16, flexWrap: "wrap", gap: 10 }}
      >
        <div className="cf-muted" style={{ fontSize: 12.5, maxWidth: "64ch" }}>
          Public address, CORS, LAN policy, and idle-session cleanup.
        </div>
        <button
          type="button"
          className="cf-btn cf-sm"
          onClick={runCheck}
          disabled={checking || !publicUrl}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2" />
            <path d="M18 4v4h-4M6 20v-4h4" />
          </svg>
          {checking ? "Checking…" : "Check connectivity"}
        </button>
      </div>

      {/* Standing reachability banner from the persisted last check. Shown
          on page load (and any time no fresh on-demand result is on screen)
          so the operator sees the last known status without re-probing. The
          richer on-demand banner below takes over once a manual check runs. */}
      {!check && lastReach && (
        <div className={`cf-banner ${lastReach.ok ? "cf-ok" : "cf-warn"}`}>
          {lastReach.ok ? (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" />
              <path d="M9 12l2 2 4-4" />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 3l9 16H3z" />
              <path d="M12 10v4M12 17v.5" />
            </svg>
          )}
          <div>
            {lastReach.ok ? "Reachable" : "Last check failed"} · checked{" "}
            {formatCheckedAgo(lastReach.checked_at)}
            {!lastReach.ok && lastReach.error ? ` — ${lastReach.error}` : ""}
          </div>
        </div>
      )}

      {/* On-demand reachability result, shown after the operator runs a
          manual check. Richer than the standing banner (latency + HTTP
          status); supersedes the standing banner above while present. */}
      {check && (
        <div className={`cf-banner ${check.ok ? "cf-ok" : "cf-warn"}`}>
          {check.ok ? (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" />
              <path d="M9 12l2 2 4-4" />
            </svg>
          ) : (
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 3l9 16H3z" />
              <path d="M12 10v4M12 17v.5" />
            </svg>
          )}
          <div>
            {check.ok ? (
              <>
                Reachable
                {check.public_url ? (
                  <>
                    {" "}at <b>{check.public_url}</b>
                  </>
                ) : null}{" "}
                in {check.latency_ms ?? "?"} ms (HTTP {check.status_code}).
              </>
            ) : (
              <>
                Not reachable: {check.error ?? "unknown error"}
                {check.latency_ms != null
                  ? ` (after ${check.latency_ms} ms)`
                  : ""}
                .
              </>
            )}
          </div>
        </div>
      )}

      {/* Production-only proxy-misconfig diagnostic. Kept verbatim in
          behaviour; restyled as a cf-banner. */}
      {diag.looks_misconfigured && (
        <div className="cf-banner cf-warn">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 3l9 16H3z" />
            <path d="M12 10v4M12 17v.5" />
          </svg>
          <div>
            <b>Trusted-proxy config looks broken.</b> This request reached the
            server from{" "}
            <span className="cf-mono">{diag.peer_ip}</span> — a private range,
            which means a reverse proxy or Docker bridge is in front.{" "}
            {diag.trusted_proxies.length === 0
              ? "TRUSTED_PROXIES is empty, so every request looks like it's coming from the proxy. Per-IP rate limits collapse to one bucket, and audit logs attribute every action to that one IP."
              : `TRUSTED_PROXIES is set to ${diag.trusted_proxies.join(", ")} — it doesn't cover ${diag.peer_ip}, so the proxy headers are ignored. Set TRUSTED_PROXIES to the CIDR your proxy lives in.`}{" "}
            See{" "}
            <a
              href="https://github.com/soybigmac/ChimpFlix/blob/main/docs/DEPLOYMENT.md#trusted-proxy-anti-patterns"
              target="_blank"
              rel="noreferrer"
            >
              the deployment runbook
            </a>{" "}
            for the exact line to add.
          </div>
        </div>
      )}

      {error && (
        <div role="status" aria-live="polite" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {/* ── Addressing ───────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Addressing</div>
            <div className="cf-sub">
              How clients and the browser reach this server.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Public URL</div>
              <div className="cf-row-help">
                The address clients use from outside the LAN. Used to build
                absolute URLs for webhooks, share links, and cast handoff, and
                for reachability checks.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="url"
                value={publicUrl}
                onChange={(e) => setPublicUrl(e.target.value)}
                placeholder="https://chimpflix.example.com"
                className={`cf-input${dirtyFields["Public URL"] ? " cf-changed" : ""}`}
                style={{ minWidth: 300 }}
              />
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">
                Bind interface
                <span className="cf-tag">restart required</span>
              </div>
              <div className="cf-row-help">
                The local socket the server listens on. Empty (default) honors
                the BIND_ADDR env; set a value like{" "}
                <span className="cf-mono">192.168.1.50:8080</span> or{" "}
                <span className="cf-mono">[::1]:8080</span> to pin the listener
                to one NIC.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="text"
                value={bindInterface}
                onChange={(e) => setBindInterface(e.target.value)}
                placeholder="0.0.0.0:8080 (or empty)"
                className={`cf-input cf-mono${bindChanged ? " cf-changed" : ""}`}
                style={{ minWidth: 200 }}
              />
              {bindChanged && (
                <span className="cf-pill cf-warn">Restart pending</span>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* ── Access policy ────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Access policy</div>
            <div className="cf-sub">Who the server answers to, and over what.</div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row cf-col">
            <div className="cf-row-main">
              <div className="cf-row-label">CORS allowlist</div>
              <div className="cf-row-help">
                Browser origins permitted to call the API. One per line. Each is
                trusted for cross-origin browser requests AND for CSRF origin
                validation, so multi-URL deployments (LAN + WAN, staging + prod)
                work without bypassing security. Leave blank to allow
                same-origin only.
              </div>
            </div>
            <div className="cf-row-control">
              <textarea
                value={origins}
                onChange={(e) => setOrigins(e.target.value)}
                rows={4}
                placeholder={"https://lan.example.com\nhttps://app.example.com"}
                className={`cf-textarea${dirtyFields["CORS allowlist"] ? " cf-changed" : ""}`}
              />
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Secure connections</div>
              <div className="cf-row-help">
                Whether plain HTTP is allowed.{" "}
                <span className="cf-mono">required</span> rejects plain-HTTP;{" "}
                <span className="cf-mono">preferred</span> allows but warns;{" "}
                <span className="cf-mono">disabled</span> permits everything.
              </div>
            </div>
            <div className="cf-row-control">
              <select
                value={secure}
                onChange={(e) =>
                  setSecure(e.target.value as SecureConnectionsMode)
                }
                className={`cf-select cf-w-auto${dirtyFields["Secure connections"] ? " cf-changed" : ""}`}
              >
                <option value="required">Required</option>
                <option value="preferred">Preferred</option>
                <option value="disabled">Disabled</option>
              </select>
            </div>
          </div>

          <div className="cf-row cf-col">
            <div className="cf-row-main">
              <div className="cf-row-label">LAN networks</div>
              <div className="cf-row-help">
                Comma-separated CIDR ranges treated as local. LAN clients skip
                the remote-stream cap and use direct addresses. Empty disables
                LAN inference.
              </div>
            </div>
            <div className="cf-row-control">
              <textarea
                value={lanNetworks}
                onChange={(e) => setLanNetworks(e.target.value)}
                rows={2}
                placeholder="192.168.0.0/16, 10.0.0.0/8"
                className={`cf-textarea${dirtyFields["LAN networks"] ? " cf-changed" : ""}`}
              />
            </div>
          </div>

          <div className="cf-row cf-col">
            <div className="cf-row-main">
              <div className="cf-row-label">Allow without auth</div>
              <div className="cf-row-help">
                Trusted LAN bypass — comma-separated CIDRs whose requests skip
                the cookie check entirely and run as the server owner. Leave
                empty unless this box is on a fully trusted network; anything
                reachable here can stream without a password.
              </div>
            </div>
            <div className="cf-row-control">
              <textarea
                value={bypassCidrs}
                onChange={(e) => setBypassCidrs(e.target.value)}
                rows={2}
                placeholder="192.168.1.50/32"
                className={`cf-textarea${dirtyFields["Allow without auth"] ? " cf-changed" : ""}`}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Sessions & streams ───────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Sessions &amp; streams</div>
            <div className="cf-sub">
              Idle cleanup and per-user concurrent stream limits.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">
                Reap idle sessions
                <span className="cf-tag">restart required</span>
              </div>
              <div className="cf-row-help">
                Drop a stream session after this much inactivity, freeing its
                transcode slot. Default 90000 (90s). Read at startup.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="number"
                min={5000}
                max={3_600_000}
                step={1000}
                value={reaperMs}
                onChange={(e) => setReaperMs(Number(e.target.value))}
                className={`cf-input cf-mono${reaperChanged ? " cf-changed" : ""}`}
                style={{ minWidth: 140 }}
              />
              <span className="cf-faint">ms</span>
              {reaperChanged && (
                <span className="cf-pill cf-warn">Restart pending</span>
              )}
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Max remote streams per user</div>
              <div className="cf-row-help">
                Concurrent off-LAN streams allowed per account. 0 = unlimited.
                LAN clients are exempt.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="number"
                min={0}
                max={64}
                value={remoteCap}
                onChange={(e) => setRemoteCap(Number(e.target.value))}
                className={`cf-input cf-mono${dirtyFields["Max remote streams"] ? " cf-changed" : ""}`}
                style={{ minWidth: 120 }}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── sticky save bar ──────────────────────────────────────────── */}
      {(dirtyCount > 0 || error || saved) && (
        <div className="cf-savebar">
          <div className="cf-sb-status">
            {error ? (
              <>
                <span className="cf-dot" style={{ background: "var(--err)" }} />
                <b style={{ color: "#fff" }}>Save failed: {error}</b>
              </>
            ) : dirtyCount > 0 ? (
              <>
                <span className="cf-dot" style={{ background: "var(--warn)" }} />
                <span>
                  <b style={{ color: "#fff" }}>
                    {dirtyCount} unsaved{" "}
                    {dirtyCount === 1 ? "change" : "changes"}
                  </b>
                  {summary && (
                    <span className="cf-faint"> · {summary}</span>
                  )}
                </span>
              </>
            ) : (
              <>
                <span className="cf-dot" style={{ background: "var(--ok)" }} />
                Saved.
              </>
            )}
          </div>
          <div className="cf-sb-actions">
            <button
              type="button"
              className="cf-btn cf-ghost cf-sm"
              onClick={discard}
              disabled={busy || dirtyCount === 0}
            >
              Discard
            </button>
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              onClick={save}
              disabled={busy || dirtyCount === 0}
            >
              {busy ? "Saving…" : "Save changes"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// Relative "Xm ago" / "Xh ago" label from an epoch-ms timestamp, for the
// standing reachability banner. Mirrors the dashboard's formatAgo phrasing.
function formatCheckedAgo(checkedAtMs: number): string {
  const deltaMs = Date.now() - checkedAtMs;
  if (deltaMs < 0 || deltaMs < 5_000) return "just now";
  const s = Math.floor(deltaMs / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
}
