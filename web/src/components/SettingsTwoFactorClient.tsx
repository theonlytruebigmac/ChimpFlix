"use client";

import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type TotpEnrollResponse,
  type TotpStatusResponse,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";
import { LoadingPlaceholder } from "./ui/LoadingPlaceholder";

type Stage =
  | { kind: "loading" }
  | { kind: "idle"; status: TotpStatusResponse }
  | {
      kind: "enrolling";
      status: TotpStatusResponse;
      enrollment: TotpEnrollResponse;
    }
  | {
      kind: "verified";
      status: TotpStatusResponse;
      recoveryCodes: string[];
    };

export function SettingsTwoFactorClient() {
  const [stage, setStage] = useState<Stage>({ kind: "loading" });
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  // Set while the RecoveryCodesPanel is up — drained into `message`
  // when the user dismisses the panel, so the success toast appears
  // *after* the codes are out of the way (not under them where it'd
  // compete for attention).
  const [pendingDismissMessage, setPendingDismissMessage] = useState<string | null>(
    null,
  );
  const [askDisable, setAskDisable] = useState(false);
  const [askRegenerate, setAskRegenerate] = useState(false);
  // Toggled by the QR `<img onError>` so the manual-entry details
  // section auto-opens (and the QR slot shows a hint) when the data
  // URL fails to render — old browsers, broken CSP, image decoder
  // hiccups all surface this way.
  const [qrLoadFailed, setQrLoadFailed] = useState(false);
  // See SettingsProfileClient for rationale — track the auto-clear
  // timer so it can be cancelled on unmount.
  const messageTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (messageTimerRef.current !== null) {
        window.clearTimeout(messageTimerRef.current);
        messageTimerRef.current = null;
      }
    };
  }, []);

  async function refresh() {
    try {
      const status = await authApi.twoFactor.status();
      setStage({ kind: "idle", status });
    } catch (e) {
      setError(parseError(e));
    }
  }

  // Mount-time fetch. Async setState landing inside .then() avoids
  // tripping react-hooks/set-state-in-effect; the cancellation guard
  // prevents writing after unmount.
  useEffect(() => {
    let cancelled = false;
    authApi.twoFactor
      .status()
      .then((status) => {
        if (cancelled) return;
        setStage({ kind: "idle", status });
      })
      .catch((e) => {
        if (cancelled) return;
        setError(parseError(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function startEnroll() {
    setError(null);
    setBusy(true);
    try {
      const enrollment = await authApi.twoFactor.enroll(password);
      const status = await authApi.twoFactor.status();
      setStage({ kind: "enrolling", status, enrollment });
      setPassword("");
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  async function verify() {
    setError(null);
    setBusy(true);
    try {
      const { recovery_codes } = await authApi.twoFactor.verify(code);
      const status = await authApi.twoFactor.status();
      setStage({ kind: "verified", status, recoveryCodes: recovery_codes });
      setCode("");
      // Defer the toast until the codes panel is dismissed — the codes
      // themselves are the success indicator while the panel is up.
      setPendingDismissMessage("Two-factor authentication enabled.");
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  async function disable() {
    setAskDisable(false);
    setError(null);
    setBusy(true);
    try {
      await authApi.twoFactor.disable(password);
      setPassword("");
      await refresh();
      setMessage("2FA disabled.");
      if (messageTimerRef.current !== null) {
        window.clearTimeout(messageTimerRef.current);
      }
      messageTimerRef.current = window.setTimeout(() => {
        messageTimerRef.current = null;
        setMessage(null);
      }, 2500);
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  async function regenerate() {
    setAskRegenerate(false);
    setError(null);
    setBusy(true);
    try {
      const { recovery_codes } = await authApi.twoFactor.regenerateRecoveryCodes(
        password,
      );
      setPassword("");
      const status = await authApi.twoFactor.status();
      setStage({ kind: "verified", status, recoveryCodes: recovery_codes });
      setPendingDismissMessage("Recovery codes regenerated.");
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  if (stage.kind === "loading") {
    return <LoadingPlaceholder />;
  }

  const status = stage.status;
  const enforcementBlocked = status.enforcement === "disabled" && !status.verified;
  const enforcementRequired = status.enforcement === "required";

  return (
    <div>
      <StatusLine status={status} />

      {message && (
        <div className="cf-banner cf-ok">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 6L9 17l-5-5" />
          </svg>
          <div>{message}</div>
        </div>
      )}
      {error && (
        <div className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {/* Step 1 — enrollment kickoff. Shown when no verified enrollment exists. */}
      {!status.verified && stage.kind === "idle" && !enforcementBlocked && (
        <div>
          <p className="cf-muted" style={{ fontSize: 12.5, marginBottom: 12 }}>
            Re-enter your password to start enrollment. You&apos;ll get an
            otpauth URI (and a base32 secret) you can paste into Google
            Authenticator, Authy, 1Password, or any RFC 6238 app.
          </p>
          <PasswordField value={password} onChange={setPassword} />
          <div className="cf-flex" style={{ marginTop: 14 }}>
            <button
              type="button"
              onClick={startEnroll}
              disabled={busy || !password}
              className="cf-btn cf-primary"
            >
              {busy ? "…" : "Begin enrollment"}
            </button>
          </div>
        </div>
      )}

      {/* Step 2 — show QR + manual secret, ask for the first code. */}
      {stage.kind === "enrolling" && (
        <div className="cf-banner cf-warn" style={{ flexDirection: "column", gap: 14 }}>
          <div>
            <b>Scan with your authenticator.</b> Open Google Authenticator /
            Authy / 1Password / any RFC 6238 app, scan this QR, then enter the
            current 6-digit code below.
          </div>
          <div className="cf-flex cf-gap16" style={{ alignItems: "flex-start", flexWrap: "wrap", width: "100%" }}>
            {qrLoadFailed ? (
              <div
                className="cf-center cf-faint"
                style={{
                  display: "flex",
                  flexDirection: "column",
                  alignItems: "center",
                  justifyContent: "center",
                  width: 200,
                  height: 200,
                  borderRadius: "var(--r)",
                  border: "1px solid var(--line-strong)",
                  fontSize: 11.5,
                  flex: "none",
                }}
              >
                <span>QR code didn&apos;t load.</span>
                <span style={{ marginTop: 4 }}>Use the manual entry secret below.</span>
              </div>
            ) : (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={stage.enrollment.qr_data_url}
                alt="TOTP enrollment QR code"
                width={200}
                height={200}
                onError={() => setQrLoadFailed(true)}
                style={{ borderRadius: "var(--r)", background: "#fff", padding: 8, flex: "none" }}
              />
            )}
            <div style={{ flex: 1, minWidth: 220 }}>
              <details open={qrLoadFailed} style={{ fontSize: 12.5 }}>
                <summary style={{ cursor: "pointer", color: "var(--fg)" }}>
                  {qrLoadFailed
                    ? "Manual entry secret (use this instead of QR)"
                    : "Can't scan? Use the manual entry secret"}
                </summary>
                <div style={{ marginTop: 8, display: "grid", gap: 10 }}>
                  <CopyBlock
                    label="Manual entry secret"
                    value={stage.enrollment.secret}
                  />
                  <CopyBlock
                    label="otpauth URI"
                    value={stage.enrollment.otpauth_uri}
                  />
                </div>
              </details>
              <div className="cf-field" style={{ marginTop: 12, marginBottom: 0 }}>
                <label className="cf-field-label">6-digit code</label>
                <input
                  type="text"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  placeholder="123 456"
                  maxLength={8}
                  className="cf-input cf-mono"
                  style={{ width: 160 }}
                />
              </div>
              <div className="cf-flex" style={{ marginTop: 12 }}>
                <button
                  type="button"
                  onClick={verify}
                  disabled={busy || !code}
                  className="cf-btn cf-primary"
                >
                  {busy ? "…" : "Verify and activate"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Step 3 — recovery codes (shown once, after verify or regenerate). */}
      {stage.kind === "verified" && (
        <RecoveryCodesPanel
          codes={stage.recoveryCodes}
          onDismiss={() => {
            // Drain the deferred success toast — now that the codes are
            // out of the way, the user sees the confirmation without it
            // competing with the codes panel for attention.
            if (pendingDismissMessage) {
              setMessage(pendingDismissMessage);
              setPendingDismissMessage(null);
            }
            void refresh();
          }}
        />
      )}

      {/* Already-enrolled controls — only meaningful when verified. */}
      {status.verified && stage.kind === "idle" && (
        <div>
          <div className="cf-row" style={{ paddingTop: 0 }}>
            <div className="cf-row-main">
              <div className="cf-row-label">Recovery codes</div>
              <div className="cf-row-help">
                <b
                  style={{
                    color:
                      status.unused_recovery_codes <= 2
                        ? "var(--warn)"
                        : "var(--fg)",
                  }}
                >
                  {status.unused_recovery_codes} of 10 remaining.
                </b>{" "}
                Regenerate to get a fresh set — old codes stop working. Requires
                your current password.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                onClick={() => setAskRegenerate(true)}
                disabled={busy || !password}
                className="cf-btn cf-sm"
              >
                {busy ? "…" : "Regenerate codes"}
              </button>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Turn off 2FA</div>
              <div className="cf-row-help">
                Requires your current password.
                {enforcementRequired
                  ? " Disabled here because the server enforces 2FA for your role."
                  : ""}
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                onClick={() => setAskDisable(true)}
                disabled={busy || !password || enforcementRequired}
                className="cf-btn cf-danger cf-sm"
              >
                {busy ? "…" : "Disable 2FA"}
              </button>
            </div>
          </div>
          <div className="cf-field" style={{ marginTop: 14, marginBottom: 0, maxWidth: 360 }}>
            <label className="cf-field-label">
              Current password (required for the actions above)
            </label>
            <input
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              maxLength={1024}
              className="cf-input"
            />
          </div>
        </div>
      )}
      {askDisable && (
        <ConfirmDialog
          title="Disable two-factor authentication?"
          body="Your existing recovery codes will be invalidated. You'll need to re-enroll to turn 2FA back on."
          confirmLabel="Disable 2FA"
          destructive
          busy={busy}
          onConfirm={() => void disable()}
          onCancel={() => setAskDisable(false)}
        />
      )}
      {askRegenerate && (
        <ConfirmDialog
          title="Generate new recovery codes?"
          body="Any unused old codes stop working immediately. Save the new codes before closing the dialog they appear in."
          confirmLabel="Regenerate"
          destructive
          busy={busy}
          onConfirm={() => void regenerate()}
          onCancel={() => setAskRegenerate(false)}
        />
      )}
    </div>
  );
}

function StatusLine({ status }: { status: TotpStatusResponse }) {
  if (status.enforcement === "disabled") {
    return (
      <div className="cf-banner cf-info">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v.5M12 11v5" />
        </svg>
        <div>2FA enrollment is disabled by the server administrator.</div>
      </div>
    );
  }
  if (status.verified) {
    const low = status.unused_recovery_codes <= 2;
    return (
      <div className={low ? "cf-banner cf-warn" : "cf-banner cf-ok"}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          {low ? (
            <>
              <path d="M12 3l9 16H3z" />
              <path d="M12 10v4M12 17v.5" />
            </>
          ) : (
            <path d="M20 6L9 17l-5-5" />
          )}
        </svg>
        <div>
          <b>Active.</b> {status.unused_recovery_codes} unused recovery{" "}
          {status.unused_recovery_codes === 1 ? "code" : "codes"} remaining.
          {low && " Regenerate soon."}
        </div>
      </div>
    );
  }
  const required = status.enforcement === "required";
  return (
    <div className={required ? "cf-banner cf-warn" : "cf-banner cf-info"}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 8v.5M12 11v5" />
      </svg>
      <div>
        {required
          ? "Two-factor is required by server policy. Enroll below before your next login."
          : "Two-factor authentication is off."}
      </div>
    </div>
  );
}

function PasswordField({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="cf-field" style={{ marginBottom: 0, maxWidth: 360 }}>
      <label className="cf-field-label">Current password</label>
      <input
        type="password"
        autoComplete="current-password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        maxLength={1024}
        className="cf-input"
      />
    </div>
  );
}

function CopyBlock({ label, value }: { label: string; value: string }) {
  // Tri-state lets us flip the button label between "Copy" / "Copied!" /
  // "Copy failed" so the user knows the click landed (or didn't —
  // permission-denied / insecure-context paths used to fail silently).
  const [state, setState] = useState<"idle" | "ok" | "err">("idle");
  const resetTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (resetTimerRef.current !== null) {
        window.clearTimeout(resetTimerRef.current);
        resetTimerRef.current = null;
      }
    };
  }, []);
  function flash(next: "ok" | "err") {
    setState(next);
    if (resetTimerRef.current !== null) window.clearTimeout(resetTimerRef.current);
    resetTimerRef.current = window.setTimeout(() => {
      resetTimerRef.current = null;
      setState("idle");
    }, 2000);
  }
  function copy() {
    navigator.clipboard
      .writeText(value)
      .then(() => flash("ok"))
      .catch(() => flash("err"));
  }
  return (
    <div>
      <div
        className="cf-flex cf-between"
        style={{ marginBottom: 5, fontSize: 11.5, color: "var(--faint)" }}
      >
        <span>{label}</span>
        <button
          type="button"
          onClick={copy}
          aria-live="polite"
          className="cf-btn cf-tiny"
        >
          {state === "ok" ? "Copied!" : state === "err" ? "Copy failed" : "Copy"}
        </button>
      </div>
      <code
        className="cf-mono"
        style={{
          display: "block",
          wordBreak: "break-all",
          borderRadius: "var(--r-sm)",
          background: "rgba(0,0,0,0.4)",
          padding: 8,
          fontSize: 11.5,
          color: "var(--fg)",
        }}
      >
        {value}
      </code>
    </div>
  );
}

function RecoveryCodesPanel({
  codes,
  onDismiss,
}: {
  codes: string[];
  onDismiss: () => void;
}) {
  const [copyState, setCopyState] = useState<"idle" | "ok" | "err">("idle");
  const copyTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
        copyTimerRef.current = null;
      }
    };
  }, []);
  function copyAll() {
    navigator.clipboard
      .writeText(codes.join("\n"))
      .then(() => {
        setCopyState("ok");
      })
      .catch(() => {
        setCopyState("err");
      })
      .finally(() => {
        if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
        copyTimerRef.current = window.setTimeout(() => {
          copyTimerRef.current = null;
          setCopyState("idle");
        }, 2500);
      });
  }
  return (
    <div className="cf-banner cf-ok" style={{ flexDirection: "column", gap: 12 }}>
      <div className="cf-flex cf-between" style={{ width: "100%" }}>
        <b>Recovery codes — save these now</b>
        <button type="button" onClick={onDismiss} className="cf-btn cf-ghost cf-tiny">
          Dismiss
        </button>
      </div>
      <div>
        Each code can be used once if you lose access to your authenticator. We
        only store hashes — these will never be shown again. Print them or stash
        them in a password manager.
      </div>
      <pre
        className="cf-mono"
        style={{
          width: "100%",
          borderRadius: "var(--r-sm)",
          background: "rgba(0,0,0,0.4)",
          padding: 12,
          fontSize: 12,
          lineHeight: 1.7,
          color: "var(--fg)",
          margin: 0,
        }}
      >
        {codes.join("\n")}
      </pre>
      <button
        type="button"
        onClick={copyAll}
        aria-live="polite"
        className="cf-btn cf-sm"
      >
        {copyState === "ok"
          ? "Copied to clipboard"
          : copyState === "err"
            ? "Copy failed — select and copy manually"
            : "Copy all"}
      </button>
    </div>
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
      // fall through
    }
    if (e.status === 429) return "Too many attempts. Try again in a moment.";
    if (e.status === 401) return "Password didn't match.";
    return `Error ${e.status}`;
  }
  return "Network error";
}
