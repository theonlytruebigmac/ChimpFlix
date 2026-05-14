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
    secure !== initial.secure_connections;

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await adminApi.network.patch({
        public_url: publicUrl.trim() || null,
        cors_origins: originsParsed,
        secure_connections: secure,
      });
      Object.assign(initial, {
        public_url: publicUrl.trim() || null,
        cors_origins: originsParsed,
        secure_connections: secure,
      });
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
          label="CORS allowed origins"
          hint="One origin per line. Used when serving cross-origin browser clients."
        >
          <textarea
            value={origins}
            onChange={(e) => setOrigins(e.target.value)}
            rows={5}
            placeholder={"https://app.example.com\nhttps://staging.example.com"}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
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

        <div className="flex items-center gap-3">
          <button
            disabled={!dirty || busy}
            onClick={save}
            className="rounded-md bg-red-500 px-4 py-2 text-sm font-semibold text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
          >
            {busy ? "Saving…" : "Save network settings"}
          </button>
          {saved && !dirty && (
            <span className="text-xs text-white/50">Saved.</span>
          )}
        </div>
      </section>
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
