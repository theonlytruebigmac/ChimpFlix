"use client";

import { useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
} from "@/lib/chimpflix-api";
import { PasswordStrengthHint } from "@/components/PasswordStrengthHint";

export function SettingsPasswordClient() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showCurrent, setShowCurrent] = useState(false);
  const [showNew, setShowNew] = useState(false);
  const [showConfirm, setShowConfirm] = useState(false);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setMessage(null);
    if (next.length < 8) {
      setError("New password must be at least 8 characters.");
      return;
    }
    if (next !== confirm) {
      setError("New password and confirmation don't match.");
      return;
    }
    setBusy(true);
    try {
      const { sessions_revoked } = await authApi.changePassword({
        current_password: current,
        new_password: next,
      });
      setCurrent("");
      setNext("");
      setConfirm("");
      setMessage(
        sessions_revoked > 0
          ? `Password updated. We signed out ${sessions_revoked} other ${sessions_revoked === 1 ? "device" : "devices"}.`
          : "Password updated.",
      );
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={onSubmit}>
      <div className="cf-grid cf-c3">
        <PasswordField
          label="Current password"
          autoComplete="current-password"
          value={current}
          onChange={setCurrent}
          show={showCurrent}
          onToggle={() => setShowCurrent((v) => !v)}
        />
        <PasswordField
          label="New password"
          autoComplete="new-password"
          value={next}
          onChange={setNext}
          show={showNew}
          onToggle={() => setShowNew((v) => !v)}
          minLength={8}
          placeholder="At least 8 characters"
        />
        <PasswordField
          label="Confirm"
          autoComplete="new-password"
          value={confirm}
          onChange={setConfirm}
          show={showConfirm}
          onToggle={() => setShowConfirm((v) => !v)}
          minLength={8}
        />
      </div>
      <PasswordStrengthHint value={next} />
      {error && (
        <div className="cf-banner cf-err" style={{ marginTop: 14 }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}
      {message && (
        <div className="cf-banner cf-ok" style={{ marginTop: 14 }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{message}</div>
        </div>
      )}
      <div className="cf-flex" style={{ marginTop: 16 }}>
        <button
          type="submit"
          disabled={busy || !current || !next || !confirm}
          className="cf-btn cf-primary"
        >
          {busy ? "Updating…" : "Change password"}
        </button>
      </div>
    </form>
  );
}

function PasswordField({
  label,
  autoComplete,
  value,
  onChange,
  show,
  onToggle,
  minLength,
  placeholder,
}: {
  label: string;
  autoComplete: "current-password" | "new-password";
  value: string;
  onChange: (v: string) => void;
  show: boolean;
  onToggle: () => void;
  minLength?: number;
  placeholder?: string;
}) {
  return (
    <div className="cf-field" style={{ marginBottom: 0 }}>
      <label className="cf-field-label">{label}</label>
      <div style={{ position: "relative" }}>
        <input
          type={show ? "text" : "password"}
          autoComplete={autoComplete}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          required
          minLength={minLength}
          maxLength={1024}
          placeholder={placeholder}
          className="cf-input"
          style={{ paddingRight: 38 }}
        />
        <button
          type="button"
          onClick={onToggle}
          aria-label={show ? "Hide password" : "Show password"}
          aria-pressed={show}
          tabIndex={-1}
          style={{
            position: "absolute",
            right: 4,
            top: "50%",
            transform: "translateY(-50%)",
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            height: 28,
            width: 28,
            borderRadius: 6,
            border: 0,
            background: "transparent",
            color: "var(--faint)",
            cursor: "pointer",
          }}
        >
          {show ? (
            <svg
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden
            >
              <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.4 18.4 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
              <line x1="1" y1="1" x2="23" y2="23" />
            </svg>
          ) : (
            <svg
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden
            >
              <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
          )}
        </button>
      </div>
    </div>
  );
}

function parseError(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    try {
      const parsed = JSON.parse(e.body) as { error?: { message?: string } };
      if (parsed.error?.message) return parsed.error.message;
    } catch {
      /* fall through */
    }
    if (e.status === 401) return "Current password didn't match.";
    return `HTTP ${e.status}`;
  }
  return "Network error";
}
