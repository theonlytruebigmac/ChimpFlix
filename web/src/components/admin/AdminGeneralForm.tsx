"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import {
  admin as adminApi,
  friendlyErrorMessage,
  type ServerSettings,
  type ServerSettingsUpdate,
  type TotpEnforcement,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

interface Props {
  initial: ServerSettings;
  /** Read-only server build version (from the settings response). */
  version: string;
  /** Read-only on-disk DATA_DIR path (from the settings response). */
  dataDir: string;
}

// 2FA enforcement maps the production enum to the mockup's Off/Optional/
// Required segmented control. `disabled` blocks new enrollments, so it
// reads as "Off" in the UI.
const TOTP_SEGMENTS: { value: TotpEnforcement; label: string }[] = [
  { value: "disabled", label: "Off" },
  { value: "optional", label: "Optional" },
  { value: "required", label: "Required" },
];

// Server-wide identity (server_name), public URL, security floor, and
// telemetry toggle. Detailed transcoder / network / webhook controls
// live in their own pages.
export function AdminGeneralForm({ initial, version, dataDir }: Props) {
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
    allow_signups: initial.allow_signups,
    totp_enforcement: initial.totp_enforcement,
  });
  const [serverName, setServerName] = useState(baseline.server_name);
  const [publicUrl, setPublicUrl] = useState(baseline.public_url ?? "");
  const [telemetry, setTelemetry] = useState(baseline.telemetry_opt_in);
  const [allowSignups, setAllowSignups] = useState(baseline.allow_signups);
  const [totpEnforcement, setTotpEnforcement] = useState<TotpEnforcement>(
    baseline.totp_enforcement,
  );
  const [error, setError] = useState<string | null>(null);
  const [saveBusy, setSaveBusy] = useState(false);
  const [rerunBusy, setRerunBusy] = useState(false);
  const [askRerun, setAskRerun] = useState(false);
  const router = useRouter();

  const dirtyFields: Record<string, boolean> = {
    "Server name": serverName.trim() !== baseline.server_name,
    "Public URL": (publicUrl || null) !== baseline.public_url,
    "Two-factor enforcement": totpEnforcement !== baseline.totp_enforcement,
    "Allow new sign-ups": allowSignups !== baseline.allow_signups,
    "Anonymous telemetry": telemetry !== baseline.telemetry_opt_in,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, isDirty]) => isDirty)
    .map(([label]) => label);
  const dirtyCount = dirtyLabels.length;

  async function save() {
    setError(null);
    setSaveBusy(true);
    const patch: ServerSettingsUpdate = {};
    const trimmedName = serverName.trim();
    if (trimmedName !== baseline.server_name) patch.server_name = trimmedName;
    if ((publicUrl || null) !== baseline.public_url) {
      patch.public_url = publicUrl.trim() || null;
    }
    if (telemetry !== baseline.telemetry_opt_in) {
      patch.telemetry_opt_in = telemetry;
    }
    if (allowSignups !== baseline.allow_signups) {
      patch.allow_signups = allowSignups;
    }
    if (totpEnforcement !== baseline.totp_enforcement) {
      patch.totp_enforcement = totpEnforcement;
    }
    try {
      await adminApi.settings.patch(patch);
      setBaseline({
        server_name: trimmedName,
        public_url: publicUrl.trim() || null,
        telemetry_opt_in: telemetry,
        allow_signups: allowSignups,
        totp_enforcement: totpEnforcement,
      });
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setSaveBusy(false);
    }
  }

  function discard() {
    setServerName(baseline.server_name);
    setPublicUrl(baseline.public_url ?? "");
    setTelemetry(baseline.telemetry_opt_in);
    setAllowSignups(baseline.allow_signups);
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
      setAskRerun(false);
      setError(friendlyErrorMessage(e));
    } finally {
      setRerunBusy(false);
    }
  }

  const savebarSummary =
    dirtyLabels.slice(0, 3).join(", ") +
    (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "");

  return (
    <div>
      {error && (
        <div
          role="status"
          aria-live="polite"
          className="cf-banner cf-err"
          style={{ marginBottom: 16 }}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {/* ── Identity ──────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Identity</div>
            <div className="cf-sub">
              What this server calls itself and where clients reach it.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Server name</div>
              <div className="cf-row-help">
                Shown to clients on the network and in the sign-in screen.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="text"
                value={serverName}
                onChange={(e) => setServerName(e.target.value)}
                className={`cf-input cf-w-auto${
                  dirtyFields["Server name"] ? " cf-changed" : ""
                }`}
                style={{ minWidth: 260 }}
                maxLength={100}
                required
              />
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Public URL</div>
              <div className="cf-row-help">
                The address remote clients and webhooks use to reach this
                server. Leave blank when serving only on a LAN.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="url"
                value={publicUrl}
                onChange={(e) => setPublicUrl(e.target.value)}
                placeholder="https://chimpflix.example.com"
                className={`cf-input cf-w-auto${
                  dirtyFields["Public URL"] ? " cf-changed" : ""
                }`}
                style={{ minWidth: 300 }}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Security ──────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Security</div>
            <div className="cf-sub">
              Account policy applied to everyone on this server.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Two-factor enforcement</div>
              <div className="cf-row-help">
                <b>Required</b> forces every account to enroll TOTP before
                login completes. <b>Optional</b> lets users enroll themselves.{" "}
                <b>Off</b> blocks new enrollments — existing 2FA stays active
                until the user disables it.
              </div>
            </div>
            <div className="cf-row-control">
              <div className="cf-seg cf-accent">
                {TOTP_SEGMENTS.map((seg) => (
                  <button
                    key={seg.value}
                    type="button"
                    className={
                      totpEnforcement === seg.value ? "cf-on" : undefined
                    }
                    onClick={() => setTotpEnforcement(seg.value)}
                  >
                    {seg.label}
                  </button>
                ))}
              </div>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Allow new sign-ups</div>
              <div className="cf-row-help">
                When on, anyone can create an account on the sign-in screen.
                Turn off to make this server <b>invite-only</b> — existing
                invites still work, but a new account can&apos;t be created
                without one.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={allowSignups}
                aria-label="Allow new sign-ups"
                className={`cf-switch${allowSignups ? " cf-on" : ""}`}
                onClick={() => setAllowSignups(!allowSignups)}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Privacy ───────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Privacy</div>
            <div className="cf-sub">What, if anything, leaves this server.</div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Anonymous telemetry</div>
              <div className="cf-row-help">
                Off by default. ChimpFlix does not collect any telemetry today —
                toggling this on records the preference so future versions can
                honor it. No media titles or personal data.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={telemetry}
                aria-label="Send anonymous usage telemetry"
                className={`cf-switch${telemetry ? " cf-on" : ""}`}
                onClick={() => setTelemetry(!telemetry)}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Setup ─────────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Setup</div>
            <div className="cf-sub">First-run configuration.</div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Version</div>
              <div className="cf-row-help">
                The ChimpFlix build this server is running.
              </div>
            </div>
            <div className="cf-row-control">
              <span className="cf-faint cf-mono">{version}</span>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Data directory</div>
              <div className="cf-row-help">
                On-disk location of the database, caches, and uploaded assets.
                Set with the <code>DATA_DIR</code> environment variable;
                changing it requires a restart.
              </div>
            </div>
            <div className="cf-row-control">
              <span className="cf-faint cf-mono">{dataDir}</span>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Re-run setup wizard</div>
              <div className="cf-row-help">
                Walk through first-run configuration again. Existing libraries,
                secrets, and settings are not touched.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                className="cf-btn cf-sm"
                onClick={() => setAskRerun(true)}
                disabled={rerunBusy}
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M14 7a4 4 0 0 0-5 5l-6 6 2 2 6-6a4 4 0 0 0 5-5l-2 2-2-2 2-2z" />
                </svg>
                {rerunBusy ? "Opening…" : "Re-run wizard"}
              </button>
            </div>
          </div>
        </div>
      </div>

      {askRerun && (
        <ConfirmDialog
          title="Re-run setup wizard?"
          body="This will redirect all users to the onboarding screen until setup is completed again. Existing libraries, secrets, and settings are not affected."
          confirmLabel="Re-run wizard"
          busy={rerunBusy}
          onConfirm={() => void rerunWizard()}
          onCancel={() => setAskRerun(false)}
        />
      )}

      {/* ── sticky save bar ───────────────────────────────────────────── */}
      {dirtyCount > 0 && (
        <div className="cf-savebar">
          <div className="cf-sb-status">
            <span className="cf-dot" style={{ background: "var(--warn)" }} />
            <span>
              <b>
                {dirtyCount} unsaved change{dirtyCount === 1 ? "" : "s"}
              </b>
              {savebarSummary && ` · ${savebarSummary}`}
            </span>
          </div>
          <div className="cf-sb-actions">
            <button
              type="button"
              className="cf-btn cf-ghost cf-sm"
              onClick={discard}
              disabled={saveBusy}
            >
              Discard
            </button>
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              onClick={save}
              disabled={saveBusy}
            >
              {saveBusy ? "Saving…" : "Save changes"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
