"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type NetworkSettings,
  type ReachabilityResult,
  type SecureConnectionsMode,
} from "@/lib/chimpflix-api";
import { Pill, SaveBar, SettingsCard, SettingsRow } from "./ui";

const INPUT_CLASS =
  "w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30";
const INPUT_CHANGED_CLASS =
  "w-full rounded-md border border-amber-400/40 bg-black/30 px-3 py-2 text-sm outline-none focus:border-amber-300";

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
  const [check, setCheck] = useState<ReachabilityResult | null>(null);
  const [checking, setChecking] = useState(false);

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

  // The reaper threshold and bind_interface are consumed at process
  // start and not hot-reloaded. Flag visibly so the operator knows why
  // their change doesn't kick in until restart.
  const reaperChanged = dirtyFields["Reap idle sessions"];
  const bindChanged = dirtyFields["Bind interface"];

  async function save() {
    setError(null);
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
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setChecking(false);
    }
  }

  const diag = initial.proxy_diagnostic;

  return (
    <div>
      {error && (
        <div className="mb-4 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {diag.looks_misconfigured && (
        <div className="mb-4 rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm text-amber-200">
          <div className="font-semibold">Trusted-proxy config looks broken</div>
          <p className="mt-1 text-xs text-amber-200/80">
            This request reached the server from{" "}
            <code className="font-mono">{diag.peer_ip}</code> — a private
            range, which means a reverse proxy or Docker bridge is in front.{" "}
            {diag.trusted_proxies.length === 0
              ? "TRUSTED_PROXIES is empty, so every request looks like it's coming from the proxy. Per-IP rate limits collapse to one bucket, and audit logs attribute every action to that one IP."
              : `TRUSTED_PROXIES is set to ${diag.trusted_proxies.join(", ")} — it doesn't cover ${diag.peer_ip}, so the proxy headers are ignored. Set TRUSTED_PROXIES to the CIDR your proxy lives in.`}
            {" "}
            See <a
              href="https://github.com/soybigmac/ChimpFlix/blob/main/docs/DEPLOYMENT.md#trusted-proxy-anti-patterns"
              target="_blank"
              rel="noreferrer"
              className="underline hover:text-amber-100"
            >
              the deployment runbook
            </a> for the exact line to add.
          </p>
        </div>
      )}

      <SettingsCard
        title="Public access"
        description="Origin used to build absolute URLs for webhooks, share links, and emails. Adjacent settings cover the bind socket and HTTPS policy."
      >
        <SettingsRow
          label="Public URL"
          help="Origin (no path) used to build absolute URLs for webhooks and share links."
          changed={dirtyFields["Public URL"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="url"
              value={publicUrl}
              onChange={(e) => setPublicUrl(e.target.value)}
              placeholder="https://chimpflix.example.com"
              className={
                dirtyFields["Public URL"]
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }
            />
            <button
              type="button"
              disabled={checking || !publicUrl}
              onClick={runCheck}
              className="rounded border border-white/15 px-3 py-2 text-sm text-white/80 hover:bg-white/5 disabled:opacity-50"
            >
              {checking ? "Checking…" : "Test"}
            </button>
          </div>
          {check && (
            <div
              className={`mt-2 rounded border px-3 py-2 text-xs ${check.ok ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300" : "border-amber-500/40 bg-amber-500/10 text-amber-300"}`}
            >
              {check.ok
                ? `Reachable in ${check.latency_ms ?? "?"} ms (HTTP ${check.status_code}).`
                : `Not reachable: ${check.error ?? "unknown error"}${check.latency_ms != null ? ` (after ${check.latency_ms} ms)` : ""}.`}
            </div>
          )}
        </SettingsRow>

        <SettingsRow
          label="Additional server URLs (CORS allowlist)"
          help="One origin per line. Each is trusted for cross-origin browser requests AND for CSRF origin validation, so multi-URL deployments (LAN URL + WAN URL, staging + prod, etc.) work without bypassing security."
          changed={dirtyFields["CORS allowlist"]}
        >
          <textarea
            value={origins}
            onChange={(e) => setOrigins(e.target.value)}
            rows={5}
            placeholder={"https://lan.example.com\nhttps://app.example.com"}
            className={
              dirtyFields["CORS allowlist"]
                ? `${INPUT_CHANGED_CLASS} font-mono`
                : `${INPUT_CLASS} font-mono`
            }
          />
        </SettingsRow>

        <SettingsRow
          label="Bind interface"
          help={
            bindChanged
              ? "Server restart required for changes to take effect."
              : "Empty (default) honors the BIND_ADDR env. Set a specific socket address like 192.168.1.50:8080 or [::1]:8080 to pin the listener to one NIC."
          }
          changed={dirtyFields["Bind interface"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={bindInterface}
              onChange={(e) => setBindInterface(e.target.value)}
              placeholder="0.0.0.0:8080 (or empty)"
              className={
                dirtyFields["Bind interface"]
                  ? `${INPUT_CHANGED_CLASS} font-mono`
                  : `${INPUT_CLASS} font-mono`
              }
            />
            {bindChanged && <Pill tone="warn">Restart pending</Pill>}
          </div>
        </SettingsRow>

        <SettingsRow
          label="Secure connections"
          help={
            <>
              <code className="font-mono">required</code> rejects plain-HTTP;{" "}
              <code className="font-mono">preferred</code> allows but warns;{" "}
              <code className="font-mono">disabled</code> permits everything.
            </>
          }
          changed={dirtyFields["Secure connections"]}
        >
          <select
            value={secure}
            onChange={(e) =>
              setSecure(e.target.value as SecureConnectionsMode)
            }
            className={
              dirtyFields["Secure connections"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            <option value="required">Required</option>
            <option value="preferred">Preferred</option>
            <option value="disabled">Disabled</option>
          </select>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="LAN policy"
        description="Define which client IPs count as “local”. Used by the per-user remote stream cap and by the auth bypass list. Empty disables the feature."
      >
        <SettingsRow
          label="LAN networks"
          help="Comma-separated CIDR list, e.g. 192.168.0.0/16, 10.0.0.0/8."
          changed={dirtyFields["LAN networks"]}
        >
          <input
            type="text"
            value={lanNetworks}
            onChange={(e) => setLanNetworks(e.target.value)}
            placeholder="192.168.0.0/16, 10.0.0.0/8"
            className={
              dirtyFields["LAN networks"]
                ? `${INPUT_CHANGED_CLASS} font-mono`
                : `${INPUT_CLASS} font-mono`
            }
          />
        </SettingsRow>
        <SettingsRow
          label="Max remote streams per user"
          help="0 = unlimited. When >0, only requests from outside `LAN networks` are counted."
          changed={dirtyFields["Max remote streams"]}
        >
          <input
            type="number"
            min={0}
            max={64}
            value={remoteCap}
            onChange={(e) => setRemoteCap(Number(e.target.value))}
            className={`w-32 tabular-nums ${
              dirtyFields["Max remote streams"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }`}
          />
        </SettingsRow>
        <SettingsRow
          label="Allow without auth"
          help="Comma-separated CIDR list. Matching IPs skip the cookie check entirely and run as the server owner. Use sparingly — only for trusted LAN automation."
          changed={dirtyFields["Allow without auth"]}
        >
          <input
            type="text"
            value={bypassCidrs}
            onChange={(e) => setBypassCidrs(e.target.value)}
            placeholder="192.168.1.50/32"
            className={
              dirtyFields["Allow without auth"]
                ? `${INPUT_CHANGED_CLASS} font-mono`
                : `${INPUT_CLASS} font-mono`
            }
          />
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Session cleanup"
        description="How aggressively the transcoder reaper kills idle sessions."
      >
        <SettingsRow
          label="Reap idle sessions after"
          help={
            reaperChanged
              ? "Server restart required for changes to take effect."
              : "ms a transcode session can go without a keepalive ping before the reaper kills it. Default 90000 (90s)."
          }
          changed={dirtyFields["Reap idle sessions"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={5000}
              max={3_600_000}
              step={1000}
              value={reaperMs}
              onChange={(e) => setReaperMs(Number(e.target.value))}
              className={`w-32 tabular-nums ${
                dirtyFields["Reap idle sessions"]
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }`}
            />
            <span className="text-sm text-white/55">ms</span>
            {reaperChanged && <Pill tone="warn">Restart pending</Pill>}
          </div>
        </SettingsRow>
      </SettingsCard>

      <SaveBar
        dirtyCount={dirtyCount}
        summary={dirtyLabels.slice(0, 3).join(", ") +
          (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "")}
        onSave={save}
        onDiscard={discard}
      />
    </div>
  );
}
