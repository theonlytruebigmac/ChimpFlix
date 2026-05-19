"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type NetworkSettings,
  type ReachabilityResult,
  type SecureConnectionsMode,
} from "@/lib/chimpflix-api";

export function AdminNetworkClient({ initial }: { initial: NetworkSettings }) {
  const [publicUrl, setPublicUrl] = useState(initial.public_url ?? "");
  const [origins, setOrigins] = useState(initial.cors_origins.join("\n"));
  const [secure, setSecure] = useState<SecureConnectionsMode>(
    initial.secure_connections,
  );
  const [reaperMs, setReaperMs] = useState(
    initial.transcoder_reaper_idle_threshold_ms,
  );
  const [remoteCap, setRemoteCap] = useState(initial.max_remote_streams_per_user);
  const [lanNetworks, setLanNetworks] = useState(initial.lan_networks);
  const [bypassCidrs, setBypassCidrs] = useState(initial.auth_bypass_cidrs);
  const [bindInterface, setBindInterface] = useState(initial.bind_interface);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [check, setCheck] = useState<ReachabilityResult | null>(null);
  const [checking, setChecking] = useState(false);

  const originsParsed = origins
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

  const dirty =
    (publicUrl || null) !== (initial.public_url ?? null) ||
    JSON.stringify(originsParsed) !== JSON.stringify(initial.cors_origins) ||
    secure !== initial.secure_connections ||
    reaperMs !== initial.transcoder_reaper_idle_threshold_ms ||
    remoteCap !== initial.max_remote_streams_per_user ||
    lanNetworks !== initial.lan_networks ||
    bypassCidrs !== initial.auth_bypass_cidrs ||
    bindInterface !== initial.bind_interface;

  // The reaper threshold is consumed at spawn time and not hot-reloaded
  // (spawn_reaper takes an i64, not a settings handle). Surface this to
  // the operator so they know why their change doesn't kick in.
  const reaperChanged =
    reaperMs !== initial.transcoder_reaper_idle_threshold_ms;
  const bindChanged = bindInterface !== initial.bind_interface;

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
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
      Object.assign(initial, patch);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
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

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <section className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="text-base font-semibold">Public access</h2>
        <Field
          label="Public URL"
          hint="Origin (no path) used to build absolute URLs for webhooks and share links."
        >
          <div className="flex items-center gap-2">
            <input
              type="url"
              value={publicUrl}
              onChange={(e) => setPublicUrl(e.target.value)}
              placeholder="https://chimpflix.example.com"
              className="flex-1 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
            <button
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
        </Field>

        <Field
          label="Additional server URLs (CORS allowlist)"
          hint="One origin per line. Each is trusted for cross-origin browser requests AND for CSRF origin validation, so multi-URL deployments (LAN URL + WAN URL, staging + prod, etc.) work without bypassing security. The canonical Public URL above is used for outgoing email/share links; this list adds extras."
        >
          <textarea
            value={origins}
            onChange={(e) => setOrigins(e.target.value)}
            rows={5}
            placeholder={"https://lan.example.com\nhttps://app.example.com"}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
        </Field>
        <Field
          label="Bind interface"
          hint={
            bindChanged
              ? "Server restart required for changes to take effect."
              : "Empty (default) honors the BIND_ADDR env (binds all interfaces). Set a specific socket address like `192.168.1.50:8080` or `[::1]:8080` to pin the listener to one NIC. Multi-NIC niche; most operators leave this empty."
          }
        >
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={bindInterface}
              onChange={(e) => setBindInterface(e.target.value)}
              placeholder="0.0.0.0:8080 (or empty)"
              className="flex-1 rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
            />
            {bindChanged && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                Restart pending
              </span>
            )}
          </div>
        </Field>

        <Field
          label="Secure connections"
          hint="`required` rejects plain-HTTP; `preferred` allows but warns; `disabled` permits everything."
        >
          <select
            value={secure}
            onChange={(e) =>
              setSecure(e.target.value as SecureConnectionsMode)
            }
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          >
            <option value="required">Required</option>
            <option value="preferred">Preferred</option>
            <option value="disabled">Disabled</option>
          </select>
        </Field>
      </section>

      <section className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-6">
        <div>
          <h2 className="text-base font-semibold">LAN policy</h2>
          <p className="mt-1 max-w-2xl text-xs text-white/55">
            Define which client IPs count as &ldquo;local&rdquo;. Used by the
            per-user remote stream cap below and by the auth bypass list. Use
            commas to separate; empty disables the feature.
          </p>
        </div>
        <Field
          label="LAN networks"
          hint="Comma-separated CIDR list, e.g. 192.168.0.0/16, 10.0.0.0/8."
        >
          <input
            type="text"
            value={lanNetworks}
            onChange={(e) => setLanNetworks(e.target.value)}
            placeholder="192.168.0.0/16, 10.0.0.0/8"
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
        </Field>
        <Field
          label="Max remote streams per user"
          hint="0 = unlimited. When >0, only requests from outside `LAN networks` are counted."
        >
          <input
            type="number"
            min={0}
            max={64}
            value={remoteCap}
            onChange={(e) => setRemoteCap(Number(e.target.value))}
            className="w-32 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
          />
        </Field>
        <Field
          label="Allow without auth"
          hint="Comma-separated CIDR list. Matching IPs skip the cookie check entirely and run as the server owner. Use sparingly — only for trusted LAN automation."
        >
          <input
            type="text"
            value={bypassCidrs}
            onChange={(e) => setBypassCidrs(e.target.value)}
            placeholder="192.168.1.50/32"
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
        </Field>
      </section>

      <section className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="text-base font-semibold">Session cleanup</h2>
        <Field
          label="Reap idle sessions after"
          hint={
            reaperChanged
              ? "Server restart required for changes to take effect."
              : "ms a transcode session can go without a keepalive ping before the reaper kills it. Default 90000 (90s)."
          }
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={5000}
              max={3_600_000}
              step={1000}
              value={reaperMs}
              onChange={(e) => setReaperMs(Number(e.target.value))}
              className="w-32 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
            />
            <span className="text-sm text-white/55">ms</span>
            {reaperChanged && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                Restart pending
              </span>
            )}
          </div>
        </Field>
      </section>

      <div className="flex items-center gap-3">
        <button
          disabled={!dirty || busy}
          onClick={save}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {busy ? "Saving…" : "Save network settings"}
        </button>
        {saved && !dirty && (
          <span className="text-xs text-white/50">Saved.</span>
        )}
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1 block text-sm font-medium">{label}</label>
      {children}
      {hint && <p className="mt-1 text-xs text-white/50">{hint}</p>}
    </div>
  );
}
