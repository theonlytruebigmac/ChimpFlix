"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  type Webhook,
  type WebhookDelivery,
  type WebhooksListResponse,
} from "@/lib/chimpflix-api";
import { Pill } from "./ui";
import { ConfirmDialog } from "../ConfirmDialog";
import { LoadingPlaceholder } from "../ui/LoadingPlaceholder";
import { formatDateTime } from "@/lib/format";

export function AdminWebhooksClient({ initial }: { initial: WebhooksListResponse }) {
  const [webhooks, setWebhooks] = useState(initial.webhooks);
  const [events] = useState(initial.events);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      const r = await adminApi.webhooks.list();
      setWebhooks(r.webhooks);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="space-y-6">
      {error && (
        <div
          role="alert"
          aria-live="assertive"
          className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300"
        >
          {error}
        </div>
      )}

      <div className="flex items-center justify-between">
        <span className="text-sm text-white/60">
          {webhooks.length} webhook{webhooks.length === 1 ? "" : "s"}
        </span>
        <button
          onClick={() => setShowAdd((v) => !v)}
          className="rounded-md bg-accent px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-accent-hover"
        >
          {showAdd ? "Cancel" : "+ New webhook"}
        </button>
      </div>

      {showAdd && (
        <NewWebhookForm
          events={events}
          onCreated={async () => {
            setShowAdd(false);
            await refresh();
          }}
          onError={setError}
        />
      )}

      <div className="space-y-3">
        {webhooks.length === 0 && !showAdd && (
          <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-8 text-center text-sm text-white/50">
            No webhooks configured.
          </div>
        )}
        {webhooks.map((w) => (
          <WebhookRow
            key={w.id}
            webhook={w}
            events={events}
            onChanged={refresh}
            onError={setError}
          />
        ))}
      </div>
    </div>
  );
}

function WebhookRow({
  webhook,
  events,
  onChanged,
  onError,
}: {
  webhook: Webhook;
  events: string[];
  onChanged: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const initialMask: string[] = (() => {
    try {
      return JSON.parse(webhook.event_mask);
    } catch {
      return [];
    }
  })();
  const [expanded, setExpanded] = useState(false);
  const [name, setName] = useState(webhook.name);
  const [url, setUrl] = useState(webhook.url);
  const [secret, setSecret] = useState(webhook.secret ?? "");
  const [mask, setMask] = useState<string[]>(initialMask);
  const [busy, setBusy] = useState(false);
  const [deliveries, setDeliveries] = useState<WebhookDelivery[] | null>(null);
  const [askDelete, setAskDelete] = useState(false);
  // Handle for the 600ms post-test deliveries refresh. Tracked so an
  // unmount mid-wait cancels the pending fetch + setState pair.
  const testRefreshTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (testRefreshTimerRef.current !== null) {
        window.clearTimeout(testRefreshTimerRef.current);
        testRefreshTimerRef.current = null;
      }
    };
  }, []);

  const dirty =
    name !== webhook.name ||
    url !== webhook.url ||
    (secret || null) !== webhook.secret ||
    JSON.stringify(mask.slice().sort()) !==
      JSON.stringify(initialMask.slice().sort());

  async function save() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.webhooks.update(webhook.id, {
        name,
        url,
        secret: secret.trim() || null,
        event_mask: mask,
      });
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function toggleEnabled() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.webhooks.update(webhook.id, { enabled: !webhook.enabled });
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function test() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.webhooks.test(webhook.id);
      if (testRefreshTimerRef.current !== null) {
        window.clearTimeout(testRefreshTimerRef.current);
      }
      testRefreshTimerRef.current = window.setTimeout(async () => {
        testRefreshTimerRef.current = null;
        if (expanded) {
          const r = await adminApi.webhooks.listDeliveries(webhook.id, 20);
          setDeliveries(r.deliveries);
        }
        setBusy(false);
      }, 600);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  }

  async function remove() {
    setAskDelete(false);
    setBusy(true);
    onError(null);
    try {
      await adminApi.webhooks.delete(webhook.id);
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function loadDeliveries() {
    if (deliveries !== null) return;
    try {
      const r = await adminApi.webhooks.listDeliveries(webhook.id, 20);
      setDeliveries(r.deliveries);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }

  function toggleMaskEntry(name: string) {
    setMask((m) =>
      m.includes(name) ? m.filter((x) => x !== name) : [...m, name],
    );
  }

  return (
    <div className="rounded-lg border border-white/10 bg-white/2">
      <div className="flex items-center gap-3 p-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm">
            <span className="font-medium">{webhook.name}</span>
            <span className="rounded bg-white/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-white/60">
              {initialMask.length} event{initialMask.length === 1 ? "" : "s"}
            </span>
          </div>
          <div className="mt-0.5 truncate font-mono text-xs text-white/40">
            {webhook.url}
          </div>
        </div>
        <button
          disabled={busy}
          onClick={test}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
        >
          Test
        </button>
        <button
          disabled={busy}
          onClick={toggleEnabled}
          className={`rounded border px-2 py-1 text-xs ${webhook.enabled ? "border-emerald-500/40 text-emerald-300" : "border-white/15 text-white/50"}`}
        >
          {webhook.enabled ? "Enabled" : "Disabled"}
        </button>
        <button
          onClick={() => {
            setExpanded((v) => !v);
            if (!expanded) loadDeliveries();
          }}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
        >
          {expanded ? "Collapse" : "Edit ▾"}
        </button>
      </div>
      {expanded && (
        <div className="space-y-4 border-t border-white/10 p-4">
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            <Field label="Name">
              <input
                type="text"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
              />
            </Field>
            <Field label="URL">
              <input
                type="url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
              />
            </Field>
            <Field
              label="Secret"
              hint="Used to sign payloads with HMAC-SHA256 (header X-ChimpFlix-Signature)."
            >
              <input
                type="text"
                value={secret}
                placeholder="(none)"
                onChange={(e) => setSecret(e.target.value)}
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
              />
            </Field>
            <Field label="Subscribed events">
              <div className="flex flex-wrap gap-1">
                {events.map((evt) => {
                  const on = mask.includes(evt);
                  return (
                    <button
                      key={evt}
                      type="button"
                      onClick={() => toggleMaskEntry(evt)}
                      className={`rounded border px-2 py-0.5 font-mono text-[11px] ${on ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-200" : "border-white/15 text-white/50"}`}
                    >
                      {evt}
                    </button>
                  );
                })}
              </div>
            </Field>
          </div>
          <div className="flex items-center gap-3">
            <button
              disabled={!dirty || busy}
              onClick={save}
              className="rounded-md bg-accent px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
            >
              Save
            </button>
            <button
              disabled={busy}
              onClick={() => setAskDelete(true)}
              className="rounded-md border border-red-500/40 px-3 py-1.5 text-sm text-red-300 hover:bg-red-500/10 disabled:opacity-50"
            >
              Delete
            </button>
          </div>

          <div>
            <div className="mb-2 text-xs uppercase tracking-wider text-white/40">
              Recent deliveries
            </div>
            {deliveries == null ? (
              <LoadingPlaceholder />
            ) : deliveries.length === 0 ? (
              <div className="text-sm text-white/40">No deliveries yet.</div>
            ) : (
              <div className="overflow-hidden rounded border border-white/10">
                <table className="w-full text-xs">
                  <thead className="bg-white/5 text-left text-white/40">
                    <tr>
                      <th className="px-3 py-1.5">When</th>
                      <th className="px-3 py-1.5">Event</th>
                      <th className="px-3 py-1.5">Status</th>
                      <th className="px-3 py-1.5">Attempts</th>
                      <th className="px-3 py-1.5">Response / error</th>
                    </tr>
                  </thead>
                  <tbody>
                    {deliveries.map((d) => {
                      const ok = d.delivered_at != null;
                      const code = d.status_code;
                      return (
                        <tr key={d.id} className="border-t border-white/5">
                          <td className="px-3 py-1.5 text-white/60">
                            {formatDateTime(d.created_at)}
                          </td>
                          <td className="px-3 py-1.5 font-mono text-white/70">
                            {d.event}
                          </td>
                          <td className="px-3 py-1.5">
                            {ok ? (
                              <Pill tone="ok">
                                Delivered{code != null ? ` ${code}` : ""}
                              </Pill>
                            ) : d.attempts >= 3 ? (
                              <Pill tone="bad">
                                Failed{code != null ? ` ${code}` : ""}
                              </Pill>
                            ) : (
                              <Pill tone="warn">Pending</Pill>
                            )}
                          </td>
                          <td className="px-3 py-1.5 tabular-nums text-white/60">
                            {d.attempts}
                          </td>
                          <td className="px-3 py-1.5 text-white/60">
                            {d.error ? (
                              <span className="text-red-300">{d.error}</span>
                            ) : (
                              <code className="line-clamp-1 font-mono">
                                {d.response_body ?? ""}
                              </code>
                            )}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      )}
      {askDelete && (
        <ConfirmDialog
          title={`Delete webhook "${webhook.name}"?`}
          body="The endpoint will stop receiving events. In-flight deliveries finish; queued retries are dropped."
          confirmLabel="Delete"
          destructive
          busy={busy}
          onConfirm={() => void remove()}
          onCancel={() => setAskDelete(false)}
        />
      )}
    </div>
  );
}

function NewWebhookForm({
  events,
  onCreated,
  onError,
}: {
  events: string[];
  onCreated: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [mask, setMask] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.webhooks.create({
        name: name.trim(),
        url: url.trim(),
        secret: secret.trim() || undefined,
        event_mask: mask,
        enabled: true,
      });
      await onCreated();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-4">
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        <Field label="Name">
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          />
        </Field>
        <Field label="URL">
          <input
            type="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="https://example.com/hook"
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          />
        </Field>
        <Field label="Secret (optional)">
          <input
            type="text"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
          />
        </Field>
        <Field label="Events">
          <div className="flex flex-wrap gap-1">
            {events.map((evt) => {
              const on = mask.includes(evt);
              return (
                <button
                  key={evt}
                  type="button"
                  onClick={() =>
                    setMask((m) =>
                      m.includes(evt) ? m.filter((x) => x !== evt) : [...m, evt],
                    )
                  }
                  className={`rounded border px-2 py-0.5 font-mono text-[11px] ${on ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-200" : "border-white/15 text-white/50"}`}
                >
                  {evt}
                </button>
              );
            })}
          </div>
        </Field>
      </div>
      <button
        disabled={busy || mask.length === 0 || !name.trim() || !url.trim()}
        onClick={submit}
        className="rounded-md bg-accent px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-accent-hover disabled:opacity-50"
      >
        {busy ? "Creating…" : "Create"}
      </button>
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
      <label className="mb-1 block text-xs font-medium uppercase tracking-wider text-white/50">
        {label}
      </label>
      {children}
      {hint && <p className="mt-1 text-xs text-white/50">{hint}</p>}
    </div>
  );
}
