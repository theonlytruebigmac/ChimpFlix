"use client";

import { Suspense, useState } from "react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { brandNameUpper } from "@/lib/env";
import { auth, ChimpFlixApiError } from "@/lib/chimpflix-api";

export default function ResetPasswordPage() {
  return (
    <Suspense fallback={null}>
      <ResetContent />
    </Suspense>
  );
}

function ResetContent() {
  const search = useSearchParams();
  const token = (search.get("token") ?? "").trim();

  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<{ sessionsRevoked: number } | null>(null);

  // Clear the inline error the moment the user starts typing in
  // either password field — without this the "passwords don't
  // match" toast lingers while they're actively fixing it, which
  // reads as "still wrong" even after they've corrected it.
  function clearErrorOnEdit() {
    if (error) setError(null);
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    if (password !== confirm) {
      setError("Passwords don't match.");
      return;
    }
    setBusy(true);
    try {
      const res = await auth.confirmPasswordReset({
        token,
        new_password: password,
      });
      setDone({ sessionsRevoked: res.sessions_revoked });
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="flex min-h-dvh items-center justify-center bg-background px-4 text-white">
      <div className="w-full max-w-sm">
        <h1 className="mb-2 text-3xl font-black tracking-tight text-(--color-accent)">
          {brandNameUpper()}
        </h1>
        <h2 className="mb-6 text-lg text-white/80">
          {done ? "Password updated" : "Choose a new password"}
        </h2>

        {!token && (
          <div className="mb-4 rounded bg-amber-500/10 px-3 py-2 text-sm text-amber-200 ring-1 ring-amber-500/30">
            This link is missing a reset token. Request a new one from{" "}
            <Link
              href="/login"
              className="underline underline-offset-2 hover:text-white"
            >
              the login page
            </Link>
            .
          </div>
        )}

        {done ? (
          <div className="space-y-3">
            <div className="rounded bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200 ring-1 ring-emerald-500/30">
              Your password has been updated.
              {done.sessionsRevoked > 0 && (
                <>
                  {" "}
                  We also signed out{" "}
                  <strong>
                    {done.sessionsRevoked} other{" "}
                    {done.sessionsRevoked === 1 ? "device" : "devices"}
                  </strong>{" "}
                  for safety.
                </>
              )}
            </div>
            <Link
              href="/login"
              className="block w-full rounded bg-(--color-accent) px-3 py-3 text-center text-base font-semibold text-white transition hover:opacity-90"
            >
              Sign in
            </Link>
          </div>
        ) : token ? (
          <form className="space-y-3" onSubmit={onSubmit}>
            <label className="block">
              <span className="mb-1 block text-sm text-white/70">New password</span>
              <input
                type="password"
                name="new-password"
                autoComplete="new-password"
                required
                minLength={8}
                maxLength={1024}
                autoFocus
                value={password}
                onChange={(e) => {
                  setPassword(e.target.value);
                  clearErrorOnEdit();
                }}
                className="w-full rounded bg-white/10 px-3 py-2.5 text-base outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
              <span className="mt-1 block text-xs text-white/40">
                At least 8 characters.
              </span>
            </label>

            <label className="block">
              <span className="mb-1 block text-sm text-white/70">Confirm password</span>
              <input
                type="password"
                name="confirm-password"
                autoComplete="new-password"
                required
                minLength={8}
                maxLength={1024}
                value={confirm}
                onChange={(e) => {
                  setConfirm(e.target.value);
                  clearErrorOnEdit();
                }}
                className="w-full rounded bg-white/10 px-3 py-2.5 text-base outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
            </label>

            {error && (
              <div className="rounded bg-red-500/10 px-3 py-2 text-sm text-red-300 ring-1 ring-red-500/30">
                {error}
              </div>
            )}

            <button
              type="submit"
              disabled={busy}
              className="w-full rounded bg-(--color-accent) px-3 py-3 text-base font-semibold text-white transition disabled:opacity-50"
            >
              {busy ? "…" : "Set new password"}
            </button>
          </form>
        ) : null}
      </div>
    </main>
  );
}

function parseError(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    try {
      const parsed = JSON.parse(e.body) as {
        error?: { code?: string; message?: string } | string;
      };
      if (parsed.error && typeof parsed.error === "object") {
        return parsed.error.message ?? `Error ${e.status}`;
      }
      if (typeof parsed.error === "string") return parsed.error;
    } catch {
      // Fall through.
    }
    if (e.status === 429) return "Too many attempts. Try again in a moment.";
    return `Error ${e.status}`;
  }
  return "Network error";
}
