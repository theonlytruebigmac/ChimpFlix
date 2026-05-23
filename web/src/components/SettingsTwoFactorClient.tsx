"use client";

import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type TotpEnrollResponse,
  type TotpStatusResponse,
} from "@/lib/chimpflix-api";
import { ConfirmDialog } from "./ConfirmDialog";

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
    return <p className="text-sm text-white/60">Loading…</p>;
  }

  const status = stage.status;
  const enforcementBlocked = status.enforcement === "disabled" && !status.verified;
  const enforcementRequired = status.enforcement === "required";

  return (
    <div className="space-y-4">
      <StatusLine status={status} />

      {error && (
        <div
          role="status"
          aria-live="polite"
          className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300"
        >
          {error}
        </div>
      )}
      {message && (
        <div
          role="status"
          aria-live="polite"
          className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200"
        >
          {message}
        </div>
      )}

      {/* Step 1 — enrollment kickoff. Shown when no verified enrollment exists. */}
      {!status.verified && stage.kind === "idle" && !enforcementBlocked && (
        <div className="space-y-3 rounded-md border border-white/10 bg-white/2 p-4">
          <h3 className="text-sm font-semibold">Set up two-factor</h3>
          <p className="text-xs text-white/55">
            Re-enter your password to start enrollment. You&apos;ll get an
            otpauth URI (and a base32 secret) you can paste into Google
            Authenticator, Authy, 1Password, or any RFC 6238 app.
          </p>
          <PasswordField value={password} onChange={setPassword} />
          <button
            type="button"
            onClick={startEnroll}
            disabled={busy || !password}
            className="rounded bg-(--color-accent) px-4 py-2.5 text-sm font-semibold text-white sm:px-3 sm:py-2 sm:text-xs disabled:opacity-50"
          >
            {busy ? "…" : "Begin enrollment"}
          </button>
        </div>
      )}

      {/* Step 2 — show QR + manual secret, ask for the first code. */}
      {stage.kind === "enrolling" && (
        <div className="space-y-4 rounded-md border border-amber-500/30 bg-amber-500/10 p-4">
          <h3 className="text-sm font-semibold text-amber-100">
            Scan with your authenticator
          </h3>
          <p className="text-xs text-amber-200/80">
            Open Google Authenticator / Authy / 1Password / any RFC 6238
            app, scan this QR, then enter the current 6-digit code below.
          </p>
          <div className="flex flex-col items-start gap-4 sm:flex-row">
            {qrLoadFailed ? (
              <div className="flex h-50 w-50 shrink-0 flex-col items-center justify-center rounded border border-amber-500/30 bg-amber-500/5 px-3 text-center text-[11px] leading-snug text-amber-200/80">
                <span>QR code didn&apos;t load.</span>
                <span className="mt-1">Use the manual entry secret below.</span>
              </div>
            ) : (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={stage.enrollment.qr_data_url}
                alt="TOTP enrollment QR code"
                width={200}
                height={200}
                onError={() => setQrLoadFailed(true)}
                className="rounded bg-white p-2"
              />
            )}
            <div className="flex-1 space-y-3">
              <details
                className="text-xs text-amber-200/80"
                open={qrLoadFailed}
              >
                <summary className="cursor-pointer text-amber-200 hover:text-amber-100">
                  {qrLoadFailed
                    ? "Manual entry secret (use this instead of QR)"
                    : "Can't scan? Use the manual entry secret"}
                </summary>
                <div className="mt-2 space-y-2">
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
              <label className="block text-xs">
                <span className="mb-1 block text-white/70">6-digit code</span>
                <input
                  type="text"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  placeholder="123 456"
                  maxLength={8}
                  className="w-40 rounded bg-white/10 px-3 py-2 font-mono text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                />
              </label>
              <button
                type="button"
                onClick={verify}
                disabled={busy || !code}
                className="rounded bg-(--color-accent) px-4 py-2.5 text-sm font-semibold text-white sm:px-3 sm:py-2 sm:text-xs disabled:opacity-50"
              >
                {busy ? "…" : "Verify and activate"}
              </button>
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
        <div className="space-y-3 rounded-md border border-white/10 bg-white/2 p-4">
          <h3 className="text-sm font-semibold">Two-factor is on</h3>
          <p className="text-xs text-white/55">
            Re-enter your password to regenerate recovery codes or
            {enforcementRequired ? " (disabling is blocked by server policy)" : " disable 2FA"}.
          </p>
          <PasswordField value={password} onChange={setPassword} />
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => setAskRegenerate(true)}
              disabled={busy || !password}
              className="rounded border border-white/15 px-3 py-2.5 text-sm font-medium text-white/80 sm:py-2 sm:text-xs hover:bg-white/5 disabled:opacity-50"
            >
              {busy ? "…" : "Regenerate recovery codes"}
            </button>
            {!enforcementRequired && (
              <button
                type="button"
                onClick={() => setAskDisable(true)}
                disabled={busy || !password}
                className="rounded border border-red-500/40 bg-red-500/10 px-3 py-2.5 text-sm font-medium text-red-200 sm:py-2 sm:text-xs hover:bg-red-500/20 disabled:opacity-50"
              >
                {busy ? "…" : "Disable 2FA"}
              </button>
            )}
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
      <div className="rounded-md border border-white/10 bg-white/3 px-3 py-2 text-xs text-white/65">
        2FA enrollment is disabled by the server administrator.
      </div>
    );
  }
  if (status.verified) {
    const tone =
      status.unused_recovery_codes <= 2
        ? "border-amber-500/40 bg-amber-500/10 text-amber-200"
        : "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
    return (
      <div className={`rounded-md border px-3 py-2 text-xs ${tone}`}>
        <strong>Active.</strong>{" "}
        {status.unused_recovery_codes} unused recovery{" "}
        {status.unused_recovery_codes === 1 ? "code" : "codes"} remaining.
        {status.unused_recovery_codes <= 2 && " Regenerate soon."}
      </div>
    );
  }
  const required = status.enforcement === "required";
  return (
    <div
      className={`rounded-md border px-3 py-2 text-xs ${
        required
          ? "border-amber-500/40 bg-amber-500/10 text-amber-200"
          : "border-white/10 bg-white/3 text-white/65"
      }`}
    >
      {required
        ? "Two-factor is required by server policy. Enroll below before your next login."
        : "Two-factor authentication is off."}
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
    <label className="block text-xs">
      <span className="mb-1 block text-white/70">Current password</span>
      <input
        type="password"
        autoComplete="current-password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        maxLength={1024}
        className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
      />
    </label>
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
      <div className="mb-1 flex items-center justify-between text-[11px] text-white/55">
        <span>{label}</span>
        <button
          type="button"
          onClick={copy}
          aria-live="polite"
          className="rounded border border-white/20 px-2 py-0.5 text-white/80 hover:bg-white/10"
        >
          {state === "ok" ? "Copied!" : state === "err" ? "Copy failed" : "Copy"}
        </button>
      </div>
      <code className="block break-all rounded bg-black/40 p-2 font-mono text-[11px] text-white/90">
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
    <div className="space-y-3 rounded-md border border-emerald-500/30 bg-emerald-500/10 p-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-emerald-100">
          Recovery codes — save these now
        </h3>
        <button
          type="button"
          onClick={onDismiss}
          className="text-xs text-emerald-200/70 hover:text-emerald-100"
        >
          Dismiss
        </button>
      </div>
      <p className="text-xs text-emerald-200/85">
        Each code can be used once if you lose access to your authenticator.
        We only store hashes — these will never be shown again. Print them or
        stash them in a password manager.
      </p>
      <pre className="rounded bg-black/40 p-3 font-mono text-[12px] leading-relaxed text-emerald-100">
        {codes.join("\n")}
      </pre>
      <button
        type="button"
        onClick={copyAll}
        aria-live="polite"
        className="rounded bg-emerald-500/30 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-500/40"
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
