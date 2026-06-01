"use client";

import { useRouter } from "next/navigation";
import { Switch } from "@/components/admin/ui";
import { useState } from "react";
import {
  admin as adminApi,
  friendlyErrorMessage,
  type ServerSettings,
  type ServerSettingsUpdate,
  type TotpEnforcement,
} from "@/lib/chimpflix-api";
import { ErrorBanner, SaveBar, SettingsCard, SettingsRow } from "./ui";

interface Props {
  initial: ServerSettings;
}

const INPUT_CLASS =
  "w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30";
const INPUT_CHANGED_CLASS =
  "w-full rounded-md border border-amber-400/40 bg-black/30 px-3 py-2 text-sm outline-none focus:border-amber-300";

// Server-wide identity (server_name), public URL, security floor, and
// telemetry toggle. Detailed transcoder / network / webhook controls
// live in their own pages.
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
  const [error, setError] = useState<string | null>(null);
  const [rerunBusy, setRerunBusy] = useState(false);
  const router = useRouter();

  const dirtyFields: Record<string, boolean> = {
    "Server name": serverName !== baseline.server_name,
    "Public URL": (publicUrl || null) !== baseline.public_url,
    "Two-factor enforcement": totpEnforcement !== baseline.totp_enforcement,
    "Anonymous telemetry": telemetry !== baseline.telemetry_opt_in,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, isDirty]) => isDirty)
    .map(([label]) => label);
  const dirtyCount = dirtyLabels.length;

  async function save() {
    setError(null);
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
    setBaseline({
      server_name: serverName,
      public_url: publicUrl.trim() || null,
      telemetry_opt_in: telemetry,
      totp_enforcement: totpEnforcement,
    });
  }

  function discard() {
    setServerName(baseline.server_name);
    setPublicUrl(baseline.public_url ?? "");
    setTelemetry(baseline.telemetry_opt_in);
    setTotpEnforcement(baseline.totp_enforcement);
  }

  async function rerunWizard() {
    setError(null);
    setRerunBusy(true);
    try {
      // Flipping the flag back to false makes the next `/` load
      // bounce into `/onboarding`. We also push there directly so
      // the operator doesn't have to click home themselves.
      await adminApi.settings.patch({ setup_completed: false });
      router.push("/onboarding");
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setRerunBusy(false);
    }
  }

  return (
    <div>
      <ErrorBanner error={error} className="mb-4" />

      <SettingsCard
        title="Identity"
        description="How the server identifies itself to clients, webhooks, and share links."
      >
        <SettingsRow
          label="Server name"
          help="Shown in the top nav and on share metadata."
          changed={dirtyFields["Server name"]}
        >
          <input
            type="text"
            value={serverName}
            onChange={(e) => setServerName(e.target.value)}
            className={
              dirtyFields["Server name"] ? INPUT_CHANGED_CLASS : INPUT_CLASS
            }
            maxLength={100}
            required
          />
        </SettingsRow>
        <SettingsRow
          label="Public URL"
          help="Used to generate absolute URLs for webhooks and share links. Leave blank when serving only on a LAN."
          changed={dirtyFields["Public URL"]}
        >
          <input
            type="url"
            value={publicUrl}
            onChange={(e) => setPublicUrl(e.target.value)}
            placeholder="https://chimpflix.example.com"
            className={
              dirtyFields["Public URL"] ? INPUT_CHANGED_CLASS : INPUT_CLASS
            }
          />
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Security"
        description="Account-level enforcement that applies to every user."
      >
        <SettingsRow
          label="Two-factor enforcement"
          help={
            <>
              <code className="font-mono">required</code> forces every account
              to enroll TOTP before login completes.{" "}
              <code className="font-mono">optional</code> lets users enroll
              themselves. <code className="font-mono">disabled</code> blocks
              new enrollments — existing 2FA stays active until the user
              disables it.
            </>
          }
          changed={dirtyFields["Two-factor enforcement"]}
        >
          <select
            value={totpEnforcement}
            onChange={(e) =>
              setTotpEnforcement(e.target.value as TotpEnforcement)
            }
            className={
              dirtyFields["Two-factor enforcement"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            <option value="optional">Optional (default)</option>
            <option value="required">Required for all users</option>
            <option value="disabled">Disabled — block new enrollments</option>
          </select>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Privacy"
        description="Controls what (if anything) leaves the server about how it's used."
      >
        <SettingsRow
          label="Anonymous telemetry"
          help="Off by default. ChimpFlix does not collect any telemetry today — toggling this on records the preference so future versions can honor it."
          changed={dirtyFields["Anonymous telemetry"]}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={telemetry} onChange={setTelemetry} />
            <span>Send anonymous usage telemetry</span>
          </label>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="First-run wizard"
        description="The onboarding tour shown on a fresh install. Replay it any time to add another library or change your metadata source."
      >
        <SettingsRow
          label="Replay onboarding"
          help="Resets the 'setup completed' flag and drops you back into the wizard. Existing libraries, secrets, and settings are not touched."
        >
          <button
            type="button"
            onClick={rerunWizard}
            disabled={rerunBusy}
            className="rounded-md border border-white/20 bg-white/5 px-3 py-1.5 text-sm font-medium text-white hover:border-white/40 hover:bg-white/10 disabled:opacity-50"
          >
            {rerunBusy ? "Opening…" : "Re-run setup wizard"}
          </button>
        </SettingsRow>
      </SettingsCard>

      <SaveBar
        dirtyCount={dirtyCount}
        summary={dirtyLabels.slice(0, 3).join(", ") +
          (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "")}
        onSave={save}
        onDiscard={discard}
      />
    </div>
  );
}
