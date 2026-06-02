"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type SecretSlotView,
  type SecretsListResponse,
  type SecretTestResponse,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { formatDateTime } from "@/lib/format";
import { TOAST_DISMISS_LONG_MS } from "@/lib/toast";

/// Slots whose vault value is a JSON object on the wire. The UI renders
/// one labeled input per field and serializes to JSON on save, so the
/// operator never has to construct `{"client_id":"…",…}` by hand. Field
/// order here matches the order the inputs appear in the editor.
type FieldSpec = {
  key: string;
  label: string;
  type?: "text" | "password";
  hint?: string;
};
const SLOT_FIELDS: Record<string, FieldSpec[]> = {
  trakt: [
    {
      key: "client_id",
      label: "Client ID",
      type: "text",
      hint: "The Client ID shown on your Trakt OAuth app page.",
    },
    {
      key: "client_secret",
      label: "Client Secret",
      type: "password",
    },
  ],
  opensubtitles: [
    { key: "api_key", label: "API Key", type: "password" },
    { key: "username", label: "Username", type: "text" },
    { key: "password", label: "Password", type: "password" },
  ],
};

/// A small refresh/sync glyph reused for the per-slot "Test" action —
/// matches the mockup's Test button icon.
function TestIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2" />
      <path d="M18 4v4h-4M6 20v-4h4" />
    </svg>
  );
}

export function AdminCredentialsClient({
  initial,
}: {
  initial: SecretsListResponse;
}) {
  const [slots, setSlots] = useState(initial.slots);
  const [encryptedAtRest] = useState(initial.encrypted_at_rest);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  function replaceSlot(next: SecretSlotView) {
    setSlots((current) =>
      current.map((s) => (s.name === next.name ? next : s)),
    );
  }

  return (
    <div>
      {/* Page title intentionally omitted — the sidebar + breadcrumb name
          the page, per the dropped-page-titles decision. */}

      {/* ── encrypted-at-rest banner ───────────────────────────────── */}
      {encryptedAtRest ? (
        <div className="cf-banner cf-ok">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" />
            <path d="M9 12l2 2 4-4" />
          </svg>
          <div>
            Secrets are encrypted at rest (
            <span className="cf-mono">CHIMPFLIX_SECRET_KEY</span> is set).
          </div>
        </div>
      ) : (
        <div className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M12 3l9 16H3z" />
            <path d="M12 10v4M12 17v.5" />
          </svg>
          <div>
            <b>Secrets are stored in plaintext.</b> Set the{" "}
            <span className="cf-mono">CHIMPFLIX_SECRET_KEY</span> environment
            variable to a 32-byte hex value (64 characters) and restart the
            server to encrypt every stored secret at rest. The server logs
            print a ready-to-paste suggestion when running without one.
          </div>
        </div>
      )}

      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}
      {notice && (
        <div
          role="status"
          aria-live="polite"
          className="cf-banner cf-ok"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{notice}</div>
        </div>
      )}

      {/* ── per-provider credential cards ──────────────────────────── */}
      {slots.map((slot) => (
        <SlotCard
          key={slot.name}
          slot={slot}
          onUpdated={replaceSlot}
          onError={setError}
        />
      ))}

      {/* ── Plex client identity (production superset) ─────────────── */}
      <PlexIdentitySection
        onError={setError}
        onNotice={(m) => {
          setNotice(m);
          // Long-form dismiss — long enough to read, short enough
          // that a stale "rotated" line doesn't sit on the page
          // through the operator's next visit.
          window.setTimeout(() => setNotice(null), TOAST_DISMISS_LONG_MS);
        }}
      />
    </div>
  );
}

/// Plex client identity — not a vault secret, but adjacent enough
/// that putting it here keeps every "long-lived per-server credential"
/// in one place. The identifier itself isn't displayed (it's a UUID
/// without operational meaning); only the rotate action lives here.
function PlexIdentitySection({
  onError,
  onNotice,
}: {
  onError: (msg: string | null) => void;
  onNotice: (msg: string) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);

  async function rotate() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.plex.rotateIdentifier();
      onNotice(
        "Plex client identifier rotated. The next sign-in flow will mint a fresh identity. Existing per-user Plex links are unaffected.",
      );
      setConfirming(false);
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
          <div className="cf-ttl">Plex client identity</div>
          <div className="cf-sub">
            Every install has a unique client identifier that ChimpFlix sends to
            Plex during PIN-based sign-in. Rotating <strong>does not</strong>{" "}
            sign out existing Plex-linked users — their per-user tokens live
            separately. It only affects future sign-in flows.
          </div>
        </div>
        <div className="cf-head-aside">
          <button
            type="button"
            className="cf-btn cf-sm"
            onClick={() => setConfirming(true)}
            disabled={busy}
          >
            {busy ? "Rotating…" : "Rotate identifier"}
          </button>
        </div>
      </div>
      <div className="cf-card-body cf-pad">
        <p className="cf-muted" style={{ fontSize: 12.5, margin: 0 }}>
          The identifier is generated on first use and reused across restarts so
          in-flight authorizations survive a redeploy. Rotate it if you suspect
          the identifier has leaked, or if you&apos;re handing the install to a
          new operator who wants a clean Plex client identity.
        </p>
      </div>
      {confirming && (
        <ConfirmDialog
          title="Rotate the Plex client identifier?"
          body={
            <>
              The next <code>/auth/plex/start</code> call will mint a fresh UUID
              and use it for all future PIN flows. Existing per-user Plex links
              are unaffected. This action is logged to the audit trail.
            </>
          }
          confirmLabel="Rotate"
          busy={busy}
          onConfirm={() => void rotate()}
          onCancel={() => setConfirming(false)}
        />
      )}
    </div>
  );
}

function SlotCard({
  slot,
  onUpdated,
  onError,
}: {
  slot: SecretSlotView;
  onUpdated: (next: SecretSlotView) => void;
  onError: (msg: string | null) => void;
}) {
  const fieldSpecs = SLOT_FIELDS[slot.name];
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<"save" | "test" | "clear" | null>(null);
  const [testResult, setTestResult] = useState<SecretTestResponse | null>(null);
  const [askClear, setAskClear] = useState(false);

  const isSet = !!slot.stored;
  const last4 = slot.stored?.last4 ?? null;

  // For multi-field slots we serialize on submit; the backend's
  // `*Creds::parse` re-parses the JSON identically to a hand-pasted
  // value, so the wire format is unchanged.
  function buildValue(): string {
    if (fieldSpecs) {
      const obj: Record<string, string> = {};
      for (const f of fieldSpecs) obj[f.key] = (fields[f.key] ?? "").trim();
      return JSON.stringify(obj);
    }
    return draft.trim();
  }

  function hasContent(): boolean {
    if (fieldSpecs) {
      return fieldSpecs.every((f) => (fields[f.key] ?? "").trim().length > 0);
    }
    return draft.trim().length > 0;
  }

  function resetDraft() {
    setDraft("");
    setFields({});
  }

  async function save() {
    if (!hasContent()) return;
    setBusy("save");
    onError(null);
    setTestResult(null);
    try {
      const next = await adminApi.secrets.set(slot.name, buildValue());
      onUpdated(next);
      resetDraft();
      setEditing(false);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function clearConfirmed() {
    setBusy("clear");
    onError(null);
    setTestResult(null);
    try {
      const next = await adminApi.secrets.clear(slot.name);
      onUpdated(next);
      setAskClear(false);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function test() {
    setBusy("test");
    onError(null);
    setTestResult(null);
    try {
      const res = await adminApi.secrets.test(
        slot.name,
        editing && hasContent() ? buildValue() : undefined,
      );
      setTestResult(res);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  // What goes in the right-hand control of the status row(s). Managed
  // (system) slots are read-only; everything else gets Edit + (when set)
  // the inline last4 readout.
  const fieldsForDisplay: FieldSpec[] = fieldSpecs ?? [
    { key: "value", label: "API key", type: "password" },
  ];

  return (
    <div className="cf-card">
      <div className="cf-card-head">
        <div>
          <div className="cf-ttl">{slot.display_name}</div>
          <div className="cf-sub">{slot.description}</div>
        </div>
        <div className="cf-head-aside">
          {isSet ? (
            <span className="cf-pill cf-ok">
              <span className="cf-dot" />
              Configured
            </span>
          ) : (
            <span className="cf-pill cf-warn">
              <span className="cf-dot" />
              Not set
            </span>
          )}
          {slot.managed ? (
            <span className="cf-tag">System</span>
          ) : (
            <>
              {isSet && (
                <button
                  className="cf-btn cf-ghost cf-sm"
                  onClick={test}
                  disabled={busy !== null}
                >
                  <TestIcon />
                  {busy === "test" ? "Testing…" : "Test"}
                </button>
              )}
              {isSet && (
                <button
                  className="cf-btn cf-ghost cf-sm cf-danger"
                  onClick={() => setAskClear(true)}
                  disabled={busy !== null}
                >
                  {busy === "clear" ? "Clearing…" : "Clear"}
                </button>
              )}
            </>
          )}
        </div>
      </div>

      <div className="cf-card-body">
        {/* Status rows: one per logical field. For multi-field slots
            (Trakt, OpenSubtitles) each field is its own row matching the
            mockup; single-field slots get one "API key" row. */}
        {!editing &&
          fieldsForDisplay.map((f, idx) => (
            <div className="cf-row" key={f.key}>
              <div className="cf-row-main">
                <div className="cf-row-label">{f.label}</div>
              </div>
              <div className="cf-row-control">
                {isSet ? (
                  // Never show cleartext — only the masked value (+ last4
                  // on the final/primary field).
                  <span className="cf-mono">
                    {"••••••••"}
                    {idx === fieldsForDisplay.length - 1 && last4
                      ? last4
                      : ""}
                  </span>
                ) : (
                  <span className="cf-faint cf-mono">Not set</span>
                )}
                {!slot.managed && idx === 0 && (
                  <button
                    className="cf-btn cf-ghost cf-sm"
                    onClick={() => {
                      setEditing(true);
                      resetDraft();
                      setTestResult(null);
                    }}
                  >
                    Edit
                  </button>
                )}
              </div>
            </div>
          ))}

        {/* Last-updated footnote (production superset over the mockup). */}
        {!editing && isSet && slot.stored && (
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-help">
                Updated {formatDateTime(slot.stored.updated_at)}
              </div>
            </div>
          </div>
        )}

        {/* ── inline editor ─────────────────────────────────────── */}
        {editing && !slot.managed && (
          <div style={{ padding: "16px 0" }}>
            {fieldSpecs ? (
              fieldSpecs.map((f, idx) => (
                <div className="cf-field" key={f.key}>
                  <label className="cf-field-label">{f.label}</label>
                  <input
                    className="cf-input cf-mono"
                    type={f.type ?? "text"}
                    value={fields[f.key] ?? ""}
                    onChange={(e) =>
                      setFields((prev) => ({
                        ...prev,
                        [f.key]: e.target.value,
                      }))
                    }
                    autoComplete="off"
                    spellCheck={false}
                    autoFocus={idx === 0}
                  />
                  {f.hint && (
                    <p
                      className="cf-faint"
                      style={{ marginTop: 6, fontSize: 11.5 }}
                    >
                      {f.hint}
                    </p>
                  )}
                </div>
              ))
            ) : (
              <div className="cf-field">
                <label className="cf-field-label">New value</label>
                <input
                  className="cf-input cf-mono"
                  type="password"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  placeholder="paste credential"
                  autoComplete="off"
                  spellCheck={false}
                  autoFocus
                />
              </div>
            )}
            <div className="cf-flex cf-gap8" style={{ marginTop: 4 }}>
              <button
                className="cf-btn cf-primary cf-sm"
                onClick={save}
                disabled={!hasContent() || busy !== null}
              >
                {busy === "save" ? "Saving…" : "Save"}
              </button>
              <button
                className="cf-btn cf-sm"
                onClick={test}
                disabled={!hasContent() || busy !== null}
              >
                <TestIcon />
                {busy === "test" ? "Testing…" : "Test before save"}
              </button>
              <button
                className="cf-btn cf-ghost cf-sm"
                onClick={() => {
                  setEditing(false);
                  resetDraft();
                  setTestResult(null);
                }}
                disabled={busy !== null}
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* ── test result ───────────────────────────────────────── */}
        {testResult && (
          <div
            className={`cf-banner ${testResult.ok ? "cf-ok" : "cf-warn"}`}
            style={{ marginTop: 12, marginBottom: 12 }}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              {testResult.ok ? (
                <path d="M20 6L9 17l-5-5" />
              ) : (
                <>
                  <path d="M12 3l9 16H3z" />
                  <path d="M12 10v4M12 17v.5" />
                </>
              )}
            </svg>
            <div>{testResult.detail}</div>
          </div>
        )}
      </div>

      {askClear && (
        <ConfirmDialog
          title={`Clear ${slot.display_name}?`}
          body={`The stored value for ${slot.display_name} will be removed from the credential vault. Anything depending on it will start failing until you set a new value.`}
          confirmLabel="Clear"
          destructive
          busy={busy === "clear"}
          onConfirm={() => void clearConfirmed()}
          onCancel={() => setAskClear(false)}
        />
      )}
    </div>
  );
}
