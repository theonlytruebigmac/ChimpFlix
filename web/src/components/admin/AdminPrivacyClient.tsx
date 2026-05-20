"use client";

import { useState } from "react";
import { admin as adminApi } from "@/lib/chimpflix-api";
import { SaveBar, SettingsCard, SettingsRow } from "./ui";

export function AdminPrivacyClient({ initial }: { initial: boolean }) {
  const [baseline, setBaseline] = useState(initial);
  const [optIn, setOptIn] = useState(baseline);
  const [error, setError] = useState<string | null>(null);

  const dirty = optIn !== baseline;

  async function save() {
    setError(null);
    const r = await adminApi.privacy.patch(optIn);
    setBaseline(r.telemetry_opt_in);
    setOptIn(r.telemetry_opt_in);
  }

  return (
    <div>
      {error && (
        <div className="mb-4 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <SettingsCard
        title="Telemetry"
        description="Anonymous usage data the server may send to help with future versions."
      >
        <SettingsRow
          label="Anonymous usage telemetry"
          help="What would be sent: server version, library counts, transcoder settings (no media titles, no user data). Nothing is sent today — this preference is recorded for future versions to honor."
          changed={dirty}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={optIn}
              onChange={(e) => setOptIn(e.target.checked)}
            />
            <span>Send anonymous usage telemetry</span>
          </label>
        </SettingsRow>
      </SettingsCard>

      <SaveBar
        dirtyCount={dirty ? 1 : 0}
        onSave={save}
        onDiscard={() => setOptIn(baseline)}
      />
    </div>
  );
}
