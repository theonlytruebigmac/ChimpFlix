"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type EmailStatusResponse,
  type SmtpSecurity,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "../ConfirmDialog";

interface Form {
  host: string;
  port: string;
  username: string;
  security: SmtpSecurity;
  fromAddress: string;
  fromName: string;
}

function formFromStatus(s: EmailStatusResponse): Form {
  return {
    host: s.smtp_host ?? "",
    port: s.smtp_port != null ? String(s.smtp_port) : "",
    username: s.smtp_username ?? "",
    security: (s.smtp_security ?? "starttls") as SmtpSecurity,
    fromAddress: s.from_address ?? "",
    fromName: s.from_name ?? "",
  };
}

export function AdminEmailClient({
  initial,
}: {
  initial: EmailStatusResponse;
}) {
  const [status, setStatus] = useState(initial);
  const [form, setForm] = useState<Form>(() => formFromStatus(initial));
  const [password, setPassword] = useState("");
  const [testEmail, setTestEmail] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [savingConfig, setSavingConfig] = useState(false);
  const [savingPassword, setSavingPassword] = useState(false);
  const [clearingPassword, setClearingPassword] = useState(false);
  const [askClearPassword, setAskClearPassword] = useState(false);
  const [testing, setTesting] = useState(false);
  const [savedNotice, setSavedNotice] = useState<string | null>(null);
  const [testNotice, setTestNotice] = useState<string | null>(null);

  const baseline = formFromStatus(status);
  const dirty =
    form.host !== baseline.host ||
    form.port !== baseline.port ||
    form.username !== baseline.username ||
    form.security !== baseline.security ||
    form.fromAddress !== baseline.fromAddress ||
    form.fromName !== baseline.fromName;

  async function saveConfig() {
    setError(null);
    setSavedNotice(null);
    setSavingConfig(true);
    try {
      const portNum = form.port.trim() === "" ? null : Number(form.port);
      if (portNum !== null && (!Number.isFinite(portNum) || portNum <= 0 || portNum > 65535)) {
        throw new Error("Port must be a number between 1 and 65535");
      }
      // Empty strings round-trip as "clear" (null) on the server.
      await adminApi.settings.patch({
        email_smtp_host: form.host.trim() || null,
        email_smtp_port: portNum,
        email_smtp_username: form.username.trim() || null,
        email_smtp_security: form.security,
        email_from_address: form.fromAddress.trim() || null,
        email_from_name: form.fromName.trim() || null,
      });
      const refreshed = await adminApi.email.status();
      setStatus(refreshed);
      setForm(formFromStatus(refreshed));
      setSavedNotice("Saved.");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingConfig(false);
    }
  }

  async function savePassword() {
    if (!password) return;
    setError(null);
    setSavedNotice(null);
    setSavingPassword(true);
    try {
      await adminApi.email.setPassword(password);
      const refreshed = await adminApi.email.status();
      setStatus(refreshed);
      setPassword("");
      setSavedNotice("Password updated.");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingPassword(false);
    }
  }

  async function clearPasswordConfirmed() {
    setError(null);
    setSavedNotice(null);
    setClearingPassword(true);
    try {
      await adminApi.email.clearPassword();
      const refreshed = await adminApi.email.status();
      setStatus(refreshed);
      setSavedNotice("Password cleared.");
      setAskClearPassword(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setClearingPassword(false);
    }
  }

  async function runTest() {
    setError(null);
    setTestNotice(null);
    setTesting(true);
    try {
      const result = await adminApi.email.test(testEmail.trim() || undefined);
      setTestNotice(result.message);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTesting(false);
    }
  }

  const portHint =
    form.security === "tls"
      ? "Implicit TLS conventionally uses 465."
      : form.security === "starttls"
        ? "STARTTLS conventionally uses 587."
        : "Plain SMTP uses 25.";

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

      <StatusBanner status={status} />

      {/* ── SMTP server ──────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">SMTP server</div>
            <div className="cf-sub">
              Used for invites, password resets, and disk alerts.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div className="cf-grid cf-c2">
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">SMTP host</label>
              <input
                className="cf-input"
                type="text"
                value={form.host}
                onChange={(e) => setForm({ ...form, host: e.target.value })}
                placeholder="smtp.example.com"
                autoComplete="off"
                spellCheck={false}
                maxLength={253}
              />
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">SMTP port</label>
              <input
                className="cf-input"
                type="number"
                value={form.port}
                onChange={(e) => setForm({ ...form, port: e.target.value })}
                placeholder="587"
                min={1}
                max={65535}
              />
              <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
                {portHint}
              </p>
            </div>
          </div>

          <div className="cf-grid cf-c2" style={{ marginTop: 16 }}>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Username</label>
              <input
                className="cf-input"
                type="text"
                value={form.username}
                onChange={(e) => setForm({ ...form, username: e.target.value })}
                placeholder="noreply@example.com"
                autoComplete="off"
                spellCheck={false}
                maxLength={256}
              />
              <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
                Leave blank for anonymous relay.
              </p>
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Security</label>
              <select
                className="cf-select"
                value={form.security}
                onChange={(e) =>
                  setForm({ ...form, security: e.target.value as SmtpSecurity })
                }
              >
                <option value="starttls">STARTTLS</option>
                <option value="tls">Implicit TLS</option>
                <option value="none">None (plain) — only for localhost</option>
              </select>
              <p className="cf-faint" style={{ marginTop: 6, fontSize: 11.5 }}>
                STARTTLS is recommended for most providers.
              </p>
            </div>
          </div>

          {/* Password — stored encrypted at rest in the credential vault. */}
          <div className="cf-field" style={{ marginTop: 16, marginBottom: 0 }}>
            <label className="cf-field-label">Password</label>
            <input
              className="cf-input"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={status.has_password ? "•••••••• (set)" : "New password"}
              autoComplete="new-password"
              spellCheck={false}
              maxLength={1024}
            />
            <div className="cf-flex cf-gap8" style={{ marginTop: 10 }}>
              <button
                type="button"
                className="cf-btn cf-sm"
                disabled={savingPassword || !password}
                onClick={savePassword}
              >
                {savingPassword
                  ? "Saving…"
                  : status.has_password
                    ? "Rotate password"
                    : "Save password"}
              </button>
              {status.has_password && (
                <button
                  type="button"
                  className="cf-btn cf-ghost cf-sm"
                  disabled={clearingPassword}
                  onClick={() => setAskClearPassword(true)}
                >
                  {clearingPassword ? "Clearing…" : "Clear"}
                </button>
              )}
            </div>
            <p className="cf-faint" style={{ marginTop: 8, fontSize: 11.5 }}>
              Stored encrypted at rest in the credential vault. The value is
              never returned by the API — to rotate, enter the new value above.
            </p>
          </div>

          <div className="cf-grid cf-c2" style={{ marginTop: 16 }}>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">From address</label>
              <input
                className="cf-input"
                type="email"
                value={form.fromAddress}
                onChange={(e) =>
                  setForm({ ...form, fromAddress: e.target.value })
                }
                placeholder="noreply@example.com"
                autoComplete="off"
                spellCheck={false}
                maxLength={320}
              />
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">From name</label>
              <input
                className="cf-input"
                type="text"
                value={form.fromName}
                onChange={(e) => setForm({ ...form, fromName: e.target.value })}
                placeholder="ChimpFlix"
                maxLength={128}
              />
            </div>
          </div>

          <div className="cf-flex cf-gap12" style={{ marginTop: 16 }}>
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              disabled={!dirty || savingConfig}
              onClick={saveConfig}
            >
              {savingConfig ? "Saving…" : "Save SMTP settings"}
            </button>
            {savedNotice && !dirty && (
              <span className="cf-faint" style={{ fontSize: 12 }}>
                {savedNotice}
              </span>
            )}
          </div>
        </div>
      </div>

      {/* ── Test ─────────────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Test</div>
            <div className="cf-sub">
              Verifies the SMTP handshake (HELO/EHLO + AUTH). Enter a recipient
              to also queue an actual test message.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div className="cf-flex cf-gap8">
            <input
              className="cf-input"
              type="email"
              value={testEmail}
              onChange={(e) => setTestEmail(e.target.value)}
              placeholder="you@example.com (optional)"
              autoComplete="off"
              spellCheck={false}
              maxLength={320}
            />
            <button
              type="button"
              className="cf-btn cf-sm"
              disabled={testing || !status.configured}
              onClick={runTest}
              title={status.configured ? "" : "Configure SMTP first"}
            >
              {testing ? "Testing…" : "Send test email"}
            </button>
          </div>
          {testNotice && (
            <div className="cf-banner cf-ok" style={{ marginTop: 16 }}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M5 12l5 5 9-11" />
              </svg>
              <div>{testNotice}</div>
            </div>
          )}
        </div>
      </div>

      {askClearPassword && (
        <ConfirmDialog
          title="Remove the stored SMTP password?"
          body="Outgoing email will fail until you set a new password. Existing invites already sent are unaffected."
          confirmLabel="Clear password"
          destructive
          busy={clearingPassword}
          onConfirm={() => void clearPasswordConfirmed()}
          onCancel={() => setAskClearPassword(false)}
        />
      )}
    </div>
  );
}

function StatusBanner({ status }: { status: EmailStatusResponse }) {
  if (!status.configured) {
    return (
      <div className="cf-banner cf-warn">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M12 3l9 16H3z" />
          <path d="M12 10v4M12 17v.5" />
        </svg>
        <div>
          Email is not configured. Invites, password resets, and admin
          notifications will fall back to non-email paths (copy-able links, no
          notifications) until you set the SMTP host + from address below.
        </div>
      </div>
    );
  }
  if (!status.has_password && status.smtp_username) {
    return (
      <div className="cf-banner cf-warn">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M12 3l9 16H3z" />
          <path d="M12 10v4M12 17v.5" />
        </svg>
        <div>
          SMTP username is set but no password is stored. The next send attempt
          will fail — set the password below.
        </div>
      </div>
    );
  }
  return (
    <div className="cf-banner cf-ok">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M5 12l5 5 9-11" />
      </svg>
      <div>
        Email is configured. Run a test below to verify connectivity
        end-to-end.
      </div>
    </div>
  );
}
