"use client";

import { useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import {
  auth as authApi,
  ChimpFlixApiError,
  type User,
} from "@/lib/chimpflix-api";

interface Props {
  initial: User;
}

/// Email change goes through a verification round-trip:
///   1. User enters new email + current password
///   2. Server emails a token to the NEW address
///   3. User clicks link → lands back here with ?verify_email=<token>
///   4. We POST it to /auth/me/email/confirm → user.email updates
export function SettingsEmailChangeClient({ initial }: Props) {
  const router = useRouter();
  const search = useSearchParams();
  const tokenFromQuery = (search.get("verify_email") ?? "").trim();

  const [currentEmail, setCurrentEmail] = useState(initial.email);
  const [newEmail, setNewEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [requestStatus, setRequestStatus] = useState<string | null>(null);
  const [verifyStatus, setVerifyStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [autoVerifying, setAutoVerifying] = useState(false);

  // If we landed here via the email link, auto-consume the token and
  // clear it from the URL so a reload doesn't double-submit.
  useEffect(() => {
    if (!tokenFromQuery || autoVerifying) return;
    setAutoVerifying(true);
    (async () => {
      try {
        const { email } = await authApi.confirmEmailChange(tokenFromQuery);
        setCurrentEmail(email);
        setVerifyStatus(`Email updated to ${email}.`);
      } catch (e) {
        setError(parseError(e));
      } finally {
        // Strip the query so it doesn't persist in history or get re-run.
        router.replace("/settings/account", { scroll: false });
      }
    })();
  }, [tokenFromQuery, autoVerifying, router]);

  async function request(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setRequestStatus(null);
    setBusy(true);
    try {
      const res = await authApi.requestEmailChange({
        new_email: newEmail.trim(),
        password,
      });
      setPassword("");
      setNewEmail("");
      setRequestStatus(
        res.email_sent
          ? `Verification link sent to ${res.new_email}. Click it within 1 hour to apply the change.`
          : `Verification token queued for ${res.new_email}, but the email wasn't delivered (SMTP not configured or send failed). Contact your admin.`,
      );
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="text-xs">
        <span className="text-white/55">Current email: </span>
        <span className="font-medium text-white">
          {currentEmail ?? <em className="text-white/50">none set</em>}
        </span>
      </div>

      {verifyStatus && (
        <div className="rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200">
          {verifyStatus}
        </div>
      )}
      {!currentEmail && (
        <FirstTimeEmailForm
          onSet={(addr) => {
            setCurrentEmail(addr);
            setVerifyStatus(`Email set to ${addr}.`);
          }}
          onError={setError}
        />
      )}

      {currentEmail && (
        <form onSubmit={request} className="space-y-3">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block text-xs">
              <span className="mb-1 block text-white/70">New email</span>
              <input
                type="email"
                autoComplete="email"
                value={newEmail}
                onChange={(e) => setNewEmail(e.target.value)}
                placeholder="new-address@example.com"
                maxLength={320}
                required
                className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
            </label>
            <label className="block text-xs">
              <span className="mb-1 block text-white/70">Current password</span>
              <input
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
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
          {requestStatus && (
            <div className="rounded border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200">
              {requestStatus}
            </div>
          )}
          <button
            type="submit"
            disabled={busy || !newEmail || !password}
            className="rounded bg-(--color-accent) px-3 py-2 text-xs font-semibold text-white disabled:opacity-50"
          >
            {busy ? "Sending…" : "Send verification email"}
          </button>
        </form>
      )}
    </div>
  );
}

/// First-time email-set form. Skips the verification round-trip because
/// there's no existing address to protect — the server's update_me
/// endpoint accepts direct writes when `users.email IS NULL`.
function FirstTimeEmailForm({
  onSet,
  onError,
}: {
  onSet: (email: string) => void;
  onError: (msg: string) => void;
}) {
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    onError("");
    setBusy(true);
    try {
      const trimmed = email.trim();
      const { user } = await authApi.updateMe({ email: trimmed });
      if (user.email) onSet(user.email);
    } catch (e) {
      onError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="space-y-3">
      <p className="text-xs text-white/55">
        Setting an email for the first time skips the verification round-trip.
        Future changes will require clicking a link sent to the new address.
      </p>
      <div className="flex gap-2">
        <input
          type="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
          maxLength={320}
          required
          className="flex-1 rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
        />
        <button
          type="submit"
          disabled={busy || !email}
          className="rounded bg-(--color-accent) px-3 py-2 text-xs font-semibold text-white disabled:opacity-50"
        >
          {busy ? "…" : "Set email"}
        </button>
      </div>
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
    if (e.status === 401) return "Password didn't match.";
    return `HTTP ${e.status}`;
  }
  return "Network error";
}
