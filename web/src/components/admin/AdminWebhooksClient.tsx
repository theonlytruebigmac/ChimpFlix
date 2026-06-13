"use client";

import { useEffect, useRef, useState } from "react";
import {
  admin as adminApi,
  type Webhook,
  type WebhookDelivery,
  type WebhookLastDelivery,
  type WebhooksListResponse,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { formatDateTime } from "@/lib/format";
import { formatRelativeAgo } from "@/lib/relative-time";

export function AdminWebhooksClient({ initial }: { initial: WebhooksListResponse }) {
  const [webhooks, setWebhooks] = useState(initial.webhooks);
  const [events] = useState(initial.events);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Snapshot "now" once after mount so the relative "Last delivery"
  // timestamps share one reference instant and stay hydration-safe
  // (server's now differs from the client's first paint). 0 until the
  // effect runs; the LastDeliveryPill renders an absolute fallback then.
  const [nowMs, setNowMs] = useState(0);
  useEffect(() => {
    setNowMs(Date.now());
  }, []); // mount-only: re-stamping on every refresh collapses relative times to "0s ago"

  async function refresh() {
    try {
      const r = await adminApi.webhooks.list();
      setWebhooks(r.webhooks);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div>
      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <div className="cf-flex cf-between" style={{ marginBottom: 14 }}>
        <div className="cf-muted" style={{ fontSize: 13 }}>
          {webhooks.length} endpoint{webhooks.length === 1 ? "" : "s"}
        </div>
        <button
          type="button"
          className="cf-btn cf-primary cf-sm"
          onClick={() => setShowAdd((v) => !v)}
        >
          {showAdd ? (
            "Cancel"
          ) : (
            <>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 5v14M5 12h14" />
              </svg>
              New webhook
            </>
          )}
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

      {webhooks.length === 0 && !showAdd ? (
        <div className="cf-card">
          <div className="cf-card-body cf-pad">
            <span className="cf-faint" style={{ fontSize: 13 }}>
              No webhooks configured.
            </span>
          </div>
        </div>
      ) : (
        webhooks.length > 0 && (
          <div className="cf-card">
            <table className="cf-table">
              <thead>
                <tr>
                  <th>URL</th>
                  <th>Events</th>
                  <th>Last delivery</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {webhooks.map((w) => (
                  <WebhookRow
                    key={w.id}
                    webhook={w}
                    events={events}
                    nowMs={nowMs}
                    onChanged={refresh}
                    onError={setError}
                  />
                ))}
              </tbody>
            </table>
          </div>
        )
      )}

      {/* ── available events ─────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Available events</div>
            <div className="cf-sub">
              Fire a POST with a JSON payload when any of these occur.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div className="cf-flex cf-wrap cf-gap8">
            {events.map((evt) => (
              <span key={evt} className="cf-tag">
                {evt}
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * At-a-glance status pill for a webhook's most recent delivery. 2xx →
 * cf-ok, any other status / transport error → cf-err, never delivered →
 * a muted "Never". Relative timestamp shares the page-wide nowMs
 * snapshot; before that snapshot exists (nowMs === 0) it falls back to an
 * absolute date so SSR/first-paint stay stable.
 */
function LastDeliveryPill({
  last,
  nowMs,
}: {
  last?: WebhookLastDelivery;
  nowMs: number;
}) {
  if (!last) {
    return (
      <span className="cf-faint" style={{ fontSize: 12 }}>
        Never
      </span>
    );
  }
  const code = last.status_code;
  // A delivery row is INSERTed at enqueue time (status_code + delivered
  // null) before the first HTTP attempt. Treat that as "Pending" (muted),
  // not a hard "Failed" — only a delivered non-2xx (or a recorded error)
  // is a real failure.
  const pending = !last.delivered && code == null;
  const ok = last.delivered && code != null && code >= 200 && code < 300;
  const tone = pending ? "" : ok ? "cf-ok" : "cf-err";
  const when =
    nowMs > 0
      ? formatRelativeAgo(last.created_at, nowMs)
      : formatDateTime(last.created_at);
  return (
    <span className="cf-flex cf-gap8" style={{ alignItems: "center" }}>
      <span className={"cf-pill " + tone}>
        <span className="cf-dot" />
        {code != null ? code : pending ? "Pending" : ok ? "Delivered" : "Failed"}
      </span>
      <span className="cf-faint" style={{ fontSize: 11.5 }} title={formatDateTime(last.created_at)}>
        {when}
      </span>
    </span>
  );
}

function WebhookRow({
  webhook,
  events,
  nowMs,
  onChanged,
  onError,
}: {
  webhook: Webhook;
  events: string[];
  nowMs: number;
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
  // Ref mirrors `expanded` so the 600ms post-test timer callback reads the
  // live value instead of the stale closure value from when test() was called.
  const expandedRef = useRef(false);
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
        // The timer callback runs OUTSIDE the surrounding try/catch, so a
        // rejected listDeliveries here would be an unhandled rejection and
        // setBusy(false) would never run — leaving the row's buttons
        // permanently disabled. Own try/finally guarantees the busy lock
        // and the timer ref are always released.
        try {
          if (expandedRef.current) {
            const r = await adminApi.webhooks.listDeliveries(webhook.id, 20);
            setDeliveries(r.deliveries);
          }
        } catch (e) {
          onError(e instanceof Error ? e.message : String(e));
        } finally {
          testRefreshTimerRef.current = null;
          setBusy(false);
        }
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
    <>
      <tr>
        <td className="cf-mono">{webhook.url}</td>
        <td>
          <span className="cf-flex cf-wrap cf-gap8">
            {initialMask.length === 0 ? (
              <span className="cf-faint" style={{ fontSize: 12 }}>
                No events
              </span>
            ) : (
              initialMask.map((evt) => (
                <span key={evt} className="cf-pill">
                  <span className="cf-dot" />
                  {evt}
                </span>
              ))
            )}
          </span>
        </td>
        <td>
          <LastDeliveryPill last={webhook.last_delivery} nowMs={nowMs} />
        </td>
        <td className="cf-num">
          <button
            type="button"
            className="cf-btn cf-ghost cf-tiny"
            onClick={() => {
              const next = !expanded;
              expandedRef.current = next;
              setExpanded(next);
              if (next) loadDeliveries();
            }}
          >
            {expanded ? "Close" : "Edit"}
          </button>
          <button
            type="button"
            className="cf-btn cf-ghost cf-tiny"
            disabled={busy}
            onClick={test}
          >
            Test
          </button>
          <button
            type="button"
            className="cf-btn cf-ghost cf-tiny cf-danger"
            disabled={busy}
            onClick={() => setAskDelete(true)}
          >
            Delete
          </button>
        </td>
      </tr>
      {expanded && (
        <tr>
          <td colSpan={4} style={{ background: "rgba(255,255,255,0.02)" }}>
            <div style={{ padding: "4px 0 8px" }}>
              <div className="cf-flex cf-between" style={{ marginBottom: 14 }}>
                <span className="cf-section-title" style={{ margin: 0 }}>
                  Edit endpoint
                </span>
                <button
                  type="button"
                  className="cf-switch-wrap cf-flex cf-gap8"
                  style={{
                    border: 0,
                    background: "transparent",
                    cursor: "pointer",
                    alignItems: "center",
                  }}
                  onClick={toggleEnabled}
                  disabled={busy}
                  aria-label={webhook.enabled ? "Disable webhook" : "Enable webhook"}
                >
                  <span className="cf-faint" style={{ fontSize: 12 }}>
                    {webhook.enabled ? "Enabled" : "Disabled"}
                  </span>
                  <span
                    role="switch"
                    aria-checked={webhook.enabled}
                    className={"cf-switch" + (webhook.enabled ? " cf-on" : "")}
                  />
                </button>
              </div>

              <div className="cf-grid cf-c2">
                <div className="cf-field">
                  <label className="cf-field-label">Name</label>
                  <input
                    className="cf-input"
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                  />
                </div>
                <div className="cf-field">
                  <label className="cf-field-label">URL</label>
                  <input
                    className="cf-input"
                    type="url"
                    value={url}
                    onChange={(e) => setUrl(e.target.value)}
                  />
                </div>
              </div>

              <div className="cf-field">
                <label className="cf-field-label">Signing secret</label>
                <input
                  className="cf-input cf-mono"
                  type="text"
                  value={secret}
                  placeholder="(none)"
                  onChange={(e) => setSecret(e.target.value)}
                />
                <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
                  Signs payloads with HMAC-SHA256 (header{" "}
                  <span className="cf-mono">X-ChimpFlix-Signature</span>).
                </p>
              </div>

              <div className="cf-field">
                <label className="cf-field-label">Subscribed events</label>
                <div className="cf-flex cf-wrap cf-gap8">
                  {events.map((evt) => {
                    const on = mask.includes(evt);
                    return (
                      <button
                        key={evt}
                        type="button"
                        onClick={() => toggleMaskEntry(evt)}
                        className={"cf-tag" + (on ? " cf-on" : "")}
                        style={
                          on
                            ? {
                                color: "#ff7a82",
                                borderColor: "var(--accent-line)",
                                background: "var(--accent-soft)",
                              }
                            : undefined
                        }
                      >
                        {evt}
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="cf-flex cf-gap8" style={{ marginTop: 4 }}>
                <button
                  type="button"
                  className="cf-btn cf-primary cf-sm"
                  disabled={!dirty || busy}
                  onClick={save}
                >
                  Save
                </button>
                <button
                  type="button"
                  className="cf-btn cf-sm"
                  disabled={busy}
                  onClick={test}
                >
                  Test
                </button>
                <button
                  type="button"
                  className="cf-btn cf-danger cf-sm"
                  disabled={busy}
                  onClick={() => setAskDelete(true)}
                >
                  Delete
                </button>
              </div>

              <div style={{ marginTop: 16 }}>
                <div className="cf-section-title" style={{ marginTop: 0 }}>
                  Recent deliveries
                </div>
                {deliveries == null ? (
                  <span className="cf-faint" style={{ fontSize: 13 }}>
                    Loading…
                  </span>
                ) : deliveries.length === 0 ? (
                  <span className="cf-faint" style={{ fontSize: 13 }}>
                    No deliveries yet.
                  </span>
                ) : (
                  <table className="cf-table">
                    <thead>
                      <tr>
                        <th>When</th>
                        <th>Event</th>
                        <th>Status</th>
                        <th>Attempts</th>
                        <th>Response / error</th>
                      </tr>
                    </thead>
                    <tbody>
                      {deliveries.map((d) => {
                        const ok = d.delivered_at != null;
                        const code = d.status_code;
                        return (
                          <tr key={d.id}>
                            <td className="cf-faint">
                              {formatDateTime(d.created_at)}
                            </td>
                            <td className="cf-mono">{d.event}</td>
                            <td>
                              {ok ? (
                                <span className="cf-pill cf-ok">
                                  <span className="cf-dot" />
                                  {code != null ? code : "Delivered"}
                                </span>
                              ) : d.attempts >= 3 ? (
                                <span className="cf-pill cf-err">
                                  <span className="cf-dot" />
                                  {code != null ? code : "Failed"}
                                </span>
                              ) : (
                                <span className="cf-pill cf-warn">
                                  <span className="cf-dot" />
                                  Pending
                                </span>
                              )}
                            </td>
                            <td className="cf-num">{d.attempts}</td>
                            <td>
                              {d.error ? (
                                <span style={{ color: "var(--err)" }}>
                                  {d.error}
                                </span>
                              ) : (
                                <span className="cf-mono">
                                  {d.response_body ?? ""}
                                </span>
                              )}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
              </div>
            </div>
          </td>
        </tr>
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
    </>
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
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">New webhook</div>
          <div className="cf-sub">
            POST a JSON payload to an endpoint when subscribed events fire.
          </div>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        <div className="cf-grid cf-c2">
          <div className="cf-field">
            <label className="cf-field-label">Name</label>
            <input
              className="cf-input"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <div className="cf-field">
            <label className="cf-field-label">URL</label>
            <input
              className="cf-input"
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://example.com/hook"
            />
          </div>
        </div>

        <div className="cf-field">
          <label className="cf-field-label">Signing secret (optional)</label>
          <input
            className="cf-input cf-mono"
            type="text"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
          />
          <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
            Signs payloads with HMAC-SHA256 (header{" "}
            <span className="cf-mono">X-ChimpFlix-Signature</span>).
          </p>
        </div>

        <div className="cf-field">
          <label className="cf-field-label">Events</label>
          <div className="cf-flex cf-wrap cf-gap8">
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
                  className={"cf-tag" + (on ? " cf-on" : "")}
                  style={
                    on
                      ? {
                          color: "#ff7a82",
                          borderColor: "var(--accent-line)",
                          background: "var(--accent-soft)",
                        }
                      : undefined
                  }
                >
                  {evt}
                </button>
              );
            })}
          </div>
        </div>

        <div className="cf-flex cf-gap8" style={{ marginTop: 4 }}>
          <button
            type="button"
            className="cf-btn cf-primary cf-sm"
            disabled={busy || mask.length === 0 || !name.trim() || !url.trim()}
            onClick={submit}
          >
            {busy ? "Creating…" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}
