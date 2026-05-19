"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ServerSettings,
  type ServerSettingsUpdate,
  type TotpEnforcement,
} from "@/lib/chimpflix-api";

interface Props {
  initial: ServerSettings;
}

// Server-wide identity (server_name), public URL, and telemetry toggle.
// Detailed transcoder / network / webhook controls live in their own pages
// once those phases land; this form intentionally only exposes the values
// that are useful to set in isolation today.
export function AdminGeneralForm({ initial }: Props) {
  // Baseline state — initialized from the `initial` prop and updated
  // on save success. Mutating `initial` directly (the previous
  // `Object.assign(initial, …)` pattern) is a React anti-pattern:
  // a parent re-render between save and the next user edit can
  // resurrect stale values, and other children holding the same
  // reference observe a silently-changed object.
  const [baseline, setBaseline] = useState({
    server_name: initial.server_name,
    public_url: initial.public_url ?? null,
    telemetry_opt_in: initial.telemetry_opt_in,
    totp_enforcement: initial.totp_enforcement,
  });
  const [serverName, setServerName] = useState(baseline.server_name);
  const [publicUrl, setPublicUrl] = useState(baseline.public_url ?? "");
  const [telemetry, setTelemetry] = useState(baseline.telemetry_opt_in);
  const [totpEnforcement, setTotpEnforcement] = useState<TotpEnforcement>(
    baseline.totp_enforcement,
  );
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const dirty =
    serverName !== baseline.server_name ||
    (publicUrl || null) !== baseline.public_url ||
    telemetry !== baseline.telemetry_opt_in ||
    totpEnforcement !== baseline.totp_enforcement;

  async function save() {
    setSaving(true);
    setError(null);
    try {
      const patch: ServerSettingsUpdate = {};
      if (serverName !== baseline.server_name) patch.server_name = serverName.trim();
      if ((publicUrl || null) !== baseline.public_url) {
        patch.public_url = publicUrl.trim() || null;
      }
      if (telemetry !== baseline.telemetry_opt_in) {
        patch.telemetry_opt_in = telemetry;
      }
      if (totpEnforcement !== baseline.totp_enforcement) {
        patch.totp_enforcement = totpEnforcement;
      }
      await adminApi.settings.patch(patch);
      setSavedAt(Date.now());
      setBaseline({
        server_name: serverName,
        public_url: publicUrl.trim() || null,
        telemetry_opt_in: telemetry,
        totp_enforcement: totpEnforcement,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (dirty && !saving) save();
      }}
      className="space-y-6"
    >
      <Section title="Identity">
        <Field label="Server name" hint="Shown in the top nav and on share metadata.">
          <input
            type="text"
            value={serverName}
            onChange={(e) => setServerName(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            maxLength={100}
            required
          />
        </Field>
        <Field
          label="Public URL"
          hint="Used to generate absolute URLs for webhooks and share links. Leave blank when serving only on a LAN."
        >
          <input
            type="url"
            value={publicUrl}
            onChange={(e) => setPublicUrl(e.target.value)}
            placeholder="https://chimpflix.example.com"
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          />
        </Field>
      </Section>

      <Section title="Security">
        <Field
          label="Two-factor enforcement"
          hint="`required` forces every account to enroll TOTP before login completes. `optional` lets users enroll themselves. `disabled` blocks new enrollments — existing 2FA stays active until the user disables it."
        >
          <select
            value={totpEnforcement}
            onChange={(e) =>
              setTotpEnforcement(e.target.value as TotpEnforcement)
            }
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          >
            <option value="optional">Optional (default)</option>
            <option value="required">Required for all users</option>
            <option value="disabled">Disabled — block new enrollments</option>
          </select>
        </Field>
      </Section>

      <Section title="Privacy">
        <label className="flex items-start gap-3">
          <input
            type="checkbox"
            checked={telemetry}
            onChange={(e) => setTelemetry(e.target.checked)}
            className="mt-1"
          />
          <div className="text-sm">
            <div className="font-medium">Send anonymous usage telemetry</div>
            <div className="mt-0.5 text-white/50">
              Off by default. ChimpFlix does not collect any telemetry today
              — toggling this on records the preference so future versions
              can honor it.
            </div>
          </div>
        </label>
      </Section>

      <div className="flex items-center gap-3">
        <button
          type="submit"
          disabled={!dirty || saving}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white transition-colors hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
        {savedAt && !dirty && !saving && (
          <span className="text-xs text-white/50">Saved.</span>
        )}
        {error && <span className="text-xs text-red-400">{error}</span>}
      </div>
    </form>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-white/2 p-6">
      <h2 className="mb-4 text-base font-semibold">{title}</h2>
      <div className="space-y-4">{children}</div>
    </section>
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
