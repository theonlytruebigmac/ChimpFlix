"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type SecretSlotView,
  type SecretsListResponse,
  type SecretTestResponse,
} from "@/lib/chimpflix-api";

export function AdminCredentialsClient({
  initial,
}: {
  initial: SecretsListResponse;
}) {
  const [slots, setSlots] = useState(initial.slots);
  const [encryptedAtRest] = useState(initial.encrypted_at_rest);
  const [error, setError] = useState<string | null>(null);

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

      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

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

function SlotCard({
  slot,
  onUpdated,
  onError,
}: {
  slot: SecretSlotView;
  onUpdated: (next: SecretSlotView) => void;
  onError: (msg: string | null) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState<"save" | "test" | "clear" | null>(null);
  const [testResult, setTestResult] = useState<SecretTestResponse | null>(null);

  const isSet = !!slot.stored;
  const last4 = slot.stored?.last4 ?? null;

  async function save() {
    if (!draft.trim()) return;
    setBusy("save");
    onError(null);
    setTestResult(null);
    try {
      const next = await adminApi.secrets.set(slot.name, draft);
      onUpdated(next);
      setDraft("");
      setEditing(false);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    if (!confirm(`Clear the stored value for ${slot.display_name}?`)) return;
    setBusy("clear");
    onError(null);
    setTestResult(null);
    try {
      const next = await adminApi.secrets.clear(slot.name);
      onUpdated(next);
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
        editing ? draft.trim() || undefined : undefined,
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
                    · updated{" "}
                    {new Date(slot.stored.updated_at).toLocaleString()}
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
                    setDraft("");
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
                  onClick={clear}
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
          <div className="mt-3 flex gap-2">
            <button
              onClick={save}
              disabled={!draft.trim() || busy !== null}
              className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
            >
              {busy === "save" ? "Saving…" : "Save"}
            </button>
            <button
              onClick={test}
              disabled={!draft.trim() || busy !== null}
              className="rounded-md bg-white/10 px-3 py-1.5 text-sm font-medium hover:bg-white/15 disabled:opacity-50"
            >
              {busy === "test" ? "Testing…" : "Test before save"}
            </button>
            <button
              onClick={() => {
                setEditing(false);
                setDraft("");
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
    </section>
  );
}
