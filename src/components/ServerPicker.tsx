"use client";

import { useEffect, useState } from "react";

export type PickerServer = {
  id: string;
  name: string;
  owned: boolean;
  presence: boolean;
  productVersion: string;
  platform: string;
  defaultUri: string;
  connections: Array<{ uri: string; local: boolean; relay: boolean }>;
};

export function ServerPicker({
  servers,
  autoSelect,
  error,
}: {
  servers: PickerServer[];
  autoSelect: boolean;
  error: string | null;
}) {
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  useEffect(() => {
    if (autoSelect && servers.length === 1) {
      // Don't pass preferUri here — let the server probe each connection
      // and pick the first that actually answers. The page-side
      // `defaultUri` is priority-only (LAN first), which is wrong when
      // the LAN address is unreachable (Docker bridge IPs, multi-NIC).
      void select(servers[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoSelect]);

  async function select(serverId: string, preferUri?: string) {
    setSubmitting(true);
    setSubmitError(null);
    try {
      const res = await fetch("/api/auth/select-server", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ serverId, preferUri }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        setSubmitError(data.error ?? `Failed (${res.status})`);
        setSubmitting(false);
        return;
      }
      window.location.href = "/";
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : String(e));
      setSubmitting(false);
    }
  }

  if (autoSelect && servers.length === 1) {
    return (
      <div className="flex min-h-screen items-center justify-center px-6">
        <p className="text-white/70">Connecting to {servers[0].name}…</p>
      </div>
    );
  }

  return (
    <div className="mx-auto flex min-h-screen max-w-3xl flex-col px-6 py-16">
      <h1 className="mb-3 text-4xl font-medium tracking-tight">
        Pick a Plex server
      </h1>
      <p className="mb-12 text-white/65">
        Servers your Plex account has access to. Pick the one whose libraries
        you want to browse — you can switch later from the menu.
      </p>

      {error && (
        <div className="mb-6 rounded-md border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          {error}
        </div>
      )}

      {servers.length === 0 ? (
        <p className="text-white/60">
          No Plex servers are reachable from this account. If you set up the
          account recently, give it a moment and refresh — Plex&apos;s resources
          API can take a minute to populate.
        </p>
      ) : (
        <ul className="flex flex-col gap-3">
          {servers.map((s) => (
            <li key={s.id}>
              <ServerCard
                server={s}
                disabled={submitting}
                onAuto={() => select(s.id)}
                onManual={(uri) => select(s.id, uri)}
              />
            </li>
          ))}
        </ul>
      )}

      {submitError && (
        <p className="mt-6 text-sm text-(--color-accent)">{submitError}</p>
      )}
    </div>
  );
}

// One row per server. Click the row to let the server auto-pick the
// fastest reachable connection (LAN → public → relay). Expand the
// chevron to see every connection Plex advertised and force a specific
// one — useful when auto-pick lands on relay but you know your direct
// path works.
function ServerCard({
  server,
  disabled,
  onAuto,
  onManual,
}: {
  server: PickerServer;
  disabled: boolean;
  onAuto: () => void;
  onManual: (uri: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="overflow-hidden rounded-md border border-white/10 bg-(--color-surface)">
      <button
        type="button"
        onClick={onAuto}
        disabled={disabled}
        className="group flex w-full items-center justify-between px-5 py-4 text-left transition-colors hover:bg-(--color-surface-elevated) disabled:opacity-50"
      >
        <div className="min-w-0">
          <div className="flex items-center gap-3">
            <span className="truncate text-lg font-semibold">
              {server.name}
            </span>
            {server.owned && (
              <span className="rounded bg-accent/20 px-1.5 py-0.5 text-xs font-medium text-(--color-accent)">
                Owned
              </span>
            )}
            {!server.presence && (
              <span className="rounded bg-white/10 px-1.5 py-0.5 text-xs font-medium text-white/60">
                Offline
              </span>
            )}
          </div>
          <div className="mt-1 truncate text-xs text-white/55">
            {server.platform} · v{server.productVersion} ·{" "}
            {server.connections.length} connection
            {server.connections.length === 1 ? "" : "s"}
          </div>
        </div>
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          className="ml-4 shrink-0 text-white/40 transition-all duration-200 group-hover:translate-x-1 group-hover:text-white"
          aria-hidden
        >
          <polyline points="9 18 15 12 9 6" />
        </svg>
      </button>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setExpanded((v) => !v);
        }}
        className="block w-full border-t border-white/5 px-5 py-2 text-left text-xs text-white/50 transition-colors hover:bg-white/3 hover:text-white/80"
      >
        {expanded ? "Hide connections" : "Show connections"}
      </button>
      {expanded && (
        <ul className="border-t border-white/5">
          {server.connections.map((c) => (
            <li key={c.uri}>
              <button
                type="button"
                onClick={() => onManual(c.uri)}
                disabled={disabled}
                className="flex w-full items-center justify-between px-5 py-2.5 text-left font-mono text-[0.7rem] text-white/65 transition-colors hover:bg-white/5 hover:text-white disabled:opacity-50"
              >
                <span className="truncate">
                  {c.uri.replace(/^https?:\/\//, "")}
                </span>
                <span className="ml-4 shrink-0 text-[0.65rem] uppercase tracking-wider text-white/35">
                  {c.relay ? "relay" : c.local ? "local" : "public"}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
