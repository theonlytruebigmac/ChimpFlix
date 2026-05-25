"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type SecretSlotView,
  type SecretsListResponse,
  type SecretTestResponse,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";
import { ErrorBanner } from "./ui";
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
    <div className="space-y-6">
      {!encryptedAtRest && (
        <div className="rounded-md border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          <div className="font-semibold">
            Secrets are stored in plaintext.
          </div>
          <div className="mt-1 text-xs text-red-200/80">
            Set the <code className="rounded bg-black/30 px-1">CHIMPFLIX_SECRET_KEY</code>{" "}
            environment variable to a 32-byte hex value (64 characters) and
            restart the server to encrypt every stored secret at rest. The
            server logs print a ready-to-paste suggestion when running
            without one.
          </div>
        </div>
      )}

      <ErrorBanner error={error} />
      {notice && (
        <div
          role="status"
          aria-live="polite"
          className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200"
        >
          {notice}
        </div>
      )}

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

      <div className="space-y-3">
        {slots.map((slot) => (
          <SlotCard
            key={slot.name}
            slot={slot}
            onUpdated={replaceSlot}
            onError={setError}
          />
        ))}
      </div>
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
    <section className="rounded-lg border border-white/10 bg-white/2 p-5">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold">Plex client identity</h3>
          <p className="mt-1 text-xs leading-relaxed text-white/55">
            Every install has a unique client identifier that ChimpFlix sends
            to Plex during PIN-based sign-in. It&apos;s generated on first
            use and reused across restarts so in-flight authorizations
            survive a redeploy. Rotate it if you suspect the identifier has
            leaked, or if you&apos;re handing the install to a new operator
            who wants a clean Plex client identity.
          </p>
          <p className="mt-2 text-xs text-white/45">
            Rotating <strong>does not</strong> sign out existing Plex-linked
            users — their per-user tokens live separately. It only affects
            future sign-in flows.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setConfirming(true)}
          disabled={busy}
          className="shrink-0 rounded-md border border-white/15 px-3 py-1.5 text-xs text-white/80 hover:border-white/30 hover:text-white disabled:opacity-50"
        >
          Rotate identifier
        </button>
      </div>
      {confirming && (
        <ConfirmDialog
          title="Rotate the Plex client identifier?"
          body={
            <>
              The next <code>/auth/plex/start</code> call will mint a fresh
              UUID and use it for all future PIN flows. Existing per-user
              Plex links are unaffected. This action is logged to the
              audit trail.
            </>
          }
          confirmLabel="Rotate"
          busy={busy}
          onConfirm={() => void rotate()}
          onCancel={() => setConfirming(false)}
        />
      )}
    </section>
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

  return (
    <section className="rounded-lg border border-white/10 bg-white/2 p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="text-base font-semibold">{slot.display_name}</h3>
            <code className="rounded bg-white/10 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-white/60">
              {slot.name}
            </code>
            {slot.managed && (
              <span className="rounded bg-blue-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-blue-200">
                System
              </span>
            )}
          </div>
          <p className="mt-1 text-xs text-white/55">{slot.description}</p>
          <div className="mt-2 text-xs text-white/40">
            {isSet ? (
              <>
                <span className="text-emerald-300">Set</span>{" "}
                <span className="font-mono">••••{last4}</span>
                {slot.stored && (
                  <span className="ml-2">
                    · updated {formatDateTime(slot.stored.updated_at)}
                  </span>
                )}
              </>
            ) : (
              <span className="text-white/40">Not set</span>
            )}
          </div>
        </div>

        {!slot.managed && (
          <div className="flex shrink-0 flex-col items-end gap-2">
            <div className="flex gap-2">
              {!editing && (
                <button
                  onClick={() => {
                    setEditing(true);
                    resetDraft();
                    setTestResult(null);
                  }}
                  className="rounded-md bg-white/10 px-2.5 py-1 text-xs font-medium hover:bg-white/15"
                >
                  {isSet ? "Replace" : "Set"}
                </button>
              )}
              {isSet && (
                <button
                  onClick={test}
                  disabled={busy !== null}
                  className="rounded-md bg-white/10 px-2.5 py-1 text-xs font-medium hover:bg-white/15 disabled:opacity-50"
                >
                  {busy === "test" ? "Testing…" : "Test"}
                </button>
              )}
              {isSet && (
                <button
                  onClick={() => setAskClear(true)}
                  disabled={busy !== null}
                  className="rounded-md border border-red-500/40 bg-red-500/10 px-2.5 py-1 text-xs font-medium text-red-200 hover:bg-red-500/15 disabled:opacity-50"
                >
                  {busy === "clear" ? "Clearing…" : "Clear"}
                </button>
              )}
            </div>
          </div>
        )}
      </div>

      {editing && !slot.managed && (
        <div className="mt-4 border-t border-white/10 pt-4">
          {fieldSpecs ? (
            <div className="space-y-3">
              {fieldSpecs.map((f, idx) => (
                <div key={f.key}>
                  <label className="mb-1 block text-xs font-medium uppercase tracking-wider text-white/50">
                    {f.label}
                  </label>
                  <input
                    type={f.type ?? "text"}
                    value={fields[f.key] ?? ""}
                    onChange={(e) =>
                      setFields((prev) => ({ ...prev, [f.key]: e.target.value }))
                    }
                    autoComplete="off"
                    spellCheck={false}
                    className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
                    autoFocus={idx === 0}
                  />
                  {f.hint && (
                    <p className="mt-1 text-[11px] text-white/40">{f.hint}</p>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <>
              <label className="mb-1 block text-xs font-medium uppercase tracking-wider text-white/50">
                New value
              </label>
              <input
                type="password"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                placeholder="paste credential"
                className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
                autoFocus
              />
            </>
          )}
          <div className="mt-3 flex gap-2">
            <button
              onClick={save}
              disabled={!hasContent() || busy !== null}
              className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
            >
              {busy === "save" ? "Saving…" : "Save"}
            </button>
            <button
              onClick={test}
              disabled={!hasContent() || busy !== null}
              className="rounded-md bg-white/10 px-3 py-1.5 text-sm font-medium hover:bg-white/15 disabled:opacity-50"
            >
              {busy === "test" ? "Testing…" : "Test before save"}
            </button>
            <button
              onClick={() => {
                setEditing(false);
                resetDraft();
                setTestResult(null);
              }}
              disabled={busy !== null}
              className="rounded-md px-3 py-1.5 text-sm text-white/60 hover:bg-white/5 hover:text-white"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {testResult && (
        <div
          className={`mt-3 rounded-md border px-3 py-2 text-xs ${
            testResult.ok
              ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"
              : "border-amber-500/30 bg-amber-500/10 text-amber-200"
          }`}
        >
          {testResult.detail}
        </div>
      )}
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
    </section>
  );
}
