"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type EmailStatusResponse,
  type SmtpSecurity,
} from "@/lib/chimpflix-api";

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

  async function clearPassword() {
    if (!confirm("Remove the stored SMTP password? Outgoing email will fail until you set a new one.")) {
      return;
    }
    setError(null);
    setSavedNotice(null);
    setClearingPassword(true);
    try {
      await adminApi.email.clearPassword();
      const refreshed = await adminApi.email.status();
      setStatus(refreshed);
      setSavedNotice("Password cleared.");
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

  return (
    <div className="space-y-6">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <StatusBanner status={status} />

      <section className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/60">
          SMTP relay
        </h2>

        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Field label="Host" hint="e.g. smtp.example.com">
            <input
              type="text"
              value={form.host}
              onChange={(e) => setForm({ ...form, host: e.target.value })}
              placeholder="smtp.example.com"
              autoComplete="off"
              spellCheck={false}
              maxLength={253}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field
            label="Port"
            hint={
              form.security === "tls"
                ? "Implicit TLS conventionally uses 465."
                : form.security === "starttls"
                  ? "STARTTLS conventionally uses 587."
                  : "Plain SMTP uses 25."
            }
          >
            <input
              type="number"
              value={form.port}
              onChange={(e) => setForm({ ...form, port: e.target.value })}
              placeholder="587"
              min={1}
              max={65535}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field label="Security" hint="STARTTLS is recommended for most providers.">
            <select
              value={form.security}
              onChange={(e) =>
                setForm({ ...form, security: e.target.value as SmtpSecurity })
              }
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            >
              <option value="starttls">STARTTLS</option>
              <option value="tls">Implicit TLS</option>
              <option value="none">None (plain) — only for localhost</option>
            </select>
          </Field>
          <Field label="Username" hint="Leave blank for anonymous relay.">
            <input
              type="text"
              value={form.username}
              onChange={(e) => setForm({ ...form, username: e.target.value })}
              placeholder="noreply@example.com"
              autoComplete="off"
              spellCheck={false}
              maxLength={256}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field label="From address" hint="What recipients see in their inbox.">
            <input
              type="email"
              value={form.fromAddress}
              onChange={(e) => setForm({ ...form, fromAddress: e.target.value })}
              placeholder="noreply@example.com"
              autoComplete="off"
              spellCheck={false}
              maxLength={320}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field label="From name" hint="Display name shown alongside the address.">
            <input
              type="text"
              value={form.fromName}
              onChange={(e) => setForm({ ...form, fromName: e.target.value })}
              placeholder="ChimpFlix"
              maxLength={128}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
        </div>

        <div className="flex items-center gap-3">
          <button
            disabled={!dirty || savingConfig}
            onClick={saveConfig}
            className="rounded-md bg-accent px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
          >
            {savingConfig ? "Saving…" : "Save SMTP settings"}
          </button>
          {savedNotice && !dirty && (
            <span className="text-xs text-white/50">{savedNotice}</span>
          )}
        </div>
      </section>

      <section className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/60">
          Password
        </h2>
        <p className="text-xs text-white/50">
          Stored encrypted at rest in the credential vault. The value is never
          returned by the API — to rotate, enter the new value below.
        </p>
        <div className="flex items-center gap-2">
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={status.has_password ? "•••••••• (set)" : "New password"}
            autoComplete="new-password"
            spellCheck={false}
            maxLength={1024}
            className="flex-1 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          />
          <button
            disabled={savingPassword || !password}
            onClick={savePassword}
            className="rounded-md bg-accent px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
          >
            {savingPassword ? "Saving…" : status.has_password ? "Rotate" : "Set"}
          </button>
          {status.has_password && (
            <button
              disabled={clearingPassword}
              onClick={clearPassword}
              className="rounded border border-white/15 px-3 py-2 text-sm text-white/70 hover:bg-white/5 disabled:opacity-50"
            >
              {clearingPassword ? "Clearing…" : "Clear"}
            </button>
          )}
        </div>
      </section>

      <section className="space-y-4 rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/60">
          Test
        </h2>
        <p className="text-xs text-white/50">
          Verifies the SMTP handshake (HELO/EHLO + AUTH) without sending. Enter
          a recipient address to also queue an actual test message.
        </p>
        <div className="flex items-center gap-2">
          <input
            type="email"
            value={testEmail}
            onChange={(e) => setTestEmail(e.target.value)}
            placeholder="you@example.com (optional)"
            autoComplete="off"
            spellCheck={false}
            maxLength={320}
            className="flex-1 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          />
          <button
            disabled={testing || !status.configured}
            onClick={runTest}
            title={status.configured ? "" : "Configure SMTP first"}
            className="rounded border border-white/15 px-3 py-2 text-sm text-white/80 hover:bg-white/5 disabled:opacity-50"
          >
            {testing ? "Testing…" : "Run test"}
          </button>
        </div>
        {testNotice && (
          <div className="rounded border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-300">
            {testNotice}
          </div>
        )}
      </section>
    </div>
  );
}

function StatusBanner({ status }: { status: EmailStatusResponse }) {
  if (!status.configured) {
    return (
      <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
        Email is not configured. Invites, password resets, and admin
        notifications will fall back to non-email paths (copy-able links, no
        notifications) until you set the SMTP host + from address below.
      </div>
    );
  }
  if (!status.has_password && status.smtp_username) {
    return (
      <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
        SMTP username is set but no password is stored. The next send attempt
        will fail — set the password below.
      </div>
    );
  }
  return (
    <div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-300">
      Email is configured. Run a test below to verify connectivity end-to-end.
    </div>
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
