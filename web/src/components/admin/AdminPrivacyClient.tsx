"use client";

import { useState } from "react";
import { admin as adminApi } from "@/lib/chimpflix-api";

export function AdminPrivacyClient({ initial }: { initial: boolean }) {
  const [optIn, setOptIn] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const r = await adminApi.privacy.patch(optIn);
      setOptIn(r.telemetry_opt_in);
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="rounded-lg border border-white/10 bg-white/2 p-6">
      <label className="flex items-start gap-3">
        <input
          type="checkbox"
          checked={optIn}
          onChange={(e) => setOptIn(e.target.checked)}
          className="mt-1"
        />
        <div className="text-sm">
          <div className="font-medium">Send anonymous usage telemetry</div>
          <div className="mt-1 text-xs text-white/50">
            What would be sent: server version, library counts, transcoder
            settings (no media titles, no user data). Nothing is sent today
            — this preference is recorded for future versions to honor.
          </div>
        </div>
      </label>
      <div className="mt-4 flex items-center gap-3">
        <button
          disabled={busy || optIn === initial}
          onClick={save}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {busy ? "Saving…" : "Save"}
        </button>
        {saved && optIn === initial && (
          <span className="text-xs text-white/50">Saved.</span>
        )}
        {error && <span className="text-xs text-red-400">{error}</span>}
      </div>
    </section>
  );
}
