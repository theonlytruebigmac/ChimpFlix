"use client";

import { useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
} from "@/lib/chimpflix-api";

export function SettingsPasswordClient() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

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
    <form onSubmit={onSubmit} className="space-y-3">
      <p className="text-xs text-white/55">
        Changing your password signs out every other device. You stay
        signed in here.
      </p>
      <div className="grid gap-3 sm:grid-cols-3">
        <label className="block text-xs">
          <span className="mb-1 block text-white/70">Current password</span>
          <input
            type="password"
            autoComplete="current-password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
            required
            maxLength={1024}
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
        </label>
        <label className="block text-xs">
          <span className="mb-1 block text-white/70">New password</span>
          <input
            type="password"
            autoComplete="new-password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            required
            minLength={8}
            maxLength={1024}
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
        </label>
        <label className="block text-xs">
          <span className="mb-1 block text-white/70">Confirm</span>
          <input
            type="password"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            required
            minLength={8}
            maxLength={1024}
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
        </label>
      </div>
      {error && (
        <div className="rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}
      {message && (
        <div className="rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200">
          {message}
        </div>
      )}
      <button
        type="submit"
        disabled={busy || !current || !next || !confirm}
        className="rounded bg-(--color-accent) px-4 py-2.5 text-sm font-semibold text-white sm:px-3 sm:py-2 sm:text-xs disabled:opacity-50"
      >
        {busy ? "Updating…" : "Change password"}
      </button>
    </form>
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
