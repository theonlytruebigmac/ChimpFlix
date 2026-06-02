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
  // clear it from the URL so a reload doesn't double-submit. The
  // setState before the async call is the "show loading state, then
  // resolve" pattern — documented exception to set-state-in-effect.
  useEffect(() => {
    if (!tokenFromQuery || autoVerifying) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setAutoVerifying(true);
    (async () => {
      let succeeded = false;
      try {
        const { email } = await authApi.confirmEmailChange(tokenFromQuery);
        setCurrentEmail(email);
        setVerifyStatus(`Email updated to ${email}.`);
        succeeded = true;
      } catch (e) {
        setError(parseError(e));
      } finally {
        // Strip the query only on success so a reload doesn't double-
        // submit the token. On failure we keep `?verify_email=...` in
        // the URL so the user can retry by reloading, or copy/share the
        // link with support — without this the failed token is gone the
        // moment they see the error.
        if (succeeded) {
          router.replace("/settings/account", { scroll: false });
        }
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
    <div>
      <div className="cf-row" style={{ paddingTop: 0 }}>
        <div className="cf-row-main">
          <div className="cf-row-label">Current email</div>
        </div>
        <div className="cf-row-control">
          {currentEmail ? (
            <span className="cf-mono">{currentEmail}</span>
          ) : (
            <span className="cf-faint">None set</span>
          )}
        </div>
      </div>

      {verifyStatus && (
        <div className="cf-banner cf-ok">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{verifyStatus}</div>
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
        <form onSubmit={request}>
          <div className="cf-grid cf-c2" style={{ marginTop: 14 }}>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">New email</label>
              <input
                type="email"
                autoComplete="email"
                value={newEmail}
                onChange={(e) => setNewEmail(e.target.value)}
                placeholder="new-address@example.com"
                maxLength={320}
                required
                className="cf-input"
              />
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Current password</label>
              <input
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                maxLength={1024}
                className="cf-input"
              />
            </div>
          </div>
          {error && (
            <div className="cf-banner cf-err" style={{ marginTop: 14 }}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <circle cx="12" cy="12" r="9" />
                <path d="M12 8v4M12 16v.5" />
              </svg>
              <div>{error}</div>
            </div>
          )}
          {requestStatus && (
            <div className="cf-banner cf-ok" style={{ marginTop: 14 }}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M20 6L9 17l-5-5" />
              </svg>
              <div>{requestStatus}</div>
            </div>
          )}
          <div className="cf-flex" style={{ marginTop: 16 }}>
            <button
              type="submit"
              disabled={busy || !newEmail || !password}
              className="cf-btn cf-primary"
            >
              {busy ? "Sending…" : "Send verification email"}
            </button>
          </div>
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
    <form onSubmit={submit}>
      <p className="cf-muted" style={{ fontSize: 12.5, margin: "10px 0" }}>
        Setting an email for the first time skips the verification round-trip.
        Future changes will require clicking a link sent to the new address.
      </p>
      <div className="cf-flex cf-gap8">
        <input
          type="email"
          autoComplete="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
          maxLength={320}
          required
          className="cf-input"
          style={{ flex: 1 }}
        />
        <button
          type="submit"
          disabled={busy || !email}
          className="cf-btn cf-primary"
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
