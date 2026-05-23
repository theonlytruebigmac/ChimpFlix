"use client";

import { Suspense, useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { brandNameUpper } from "@/lib/env";
import { auth, ChimpFlixApiError, safeLocalPath } from "@/lib/chimpflix-api";
import { PlexSignInButton } from "@/components/PlexSignInButton";

type Mode = "login" | "setup" | "register" | "forgot" | "two_factor";

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginContent />
    </Suspense>
  );
}

function LoginContent() {
  const router = useRouter();
  const search = useSearchParams();
  // The `?next=` parameter is user-controlled: an attacker can craft
  // `/login?next=https://attacker.com` and Next's router will happily
  // navigate off-origin after a successful login. `safeLocalPath`
  // enforces "must start with a single `/`, not protocol-relative,
  // no control chars, no backslash-trick" so we only redirect to
  // known-local routes.
  const next = safeLocalPath(search.get("next"), "/");
  const invite = search.get("invite")?.trim() ?? "";

  // If we landed with an invite code in the URL we know the user wants
  // to register; otherwise wait for the server to tell us whether to
  // show setup (first-run) or normal login. Initialising from the
  // search param here avoids a setState-in-effect for the deterministic
  // case (no extra render, no React purity violation).
  const [mode, setMode] = useState<Mode | null>(invite ? "register" : null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [email, setEmail] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [useRecovery, setUseRecovery] = useState(false);
  const [challenge, setChallenge] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // First-run detection. The invite-present path is handled at useState
  // init above; here we only need the async server status check when
  // mode is still null (no invite, server hasn't replied yet).
  //
  // Also catches "already signed in" — bookmarking /login while
  // authenticated used to land on the login form regardless, which
  // looks broken. We redirect to `next` instead. Invite-bearing links
  // are an exception: the user is being asked to register a new
  // account, so we keep the form even if a different session is
  // already active (the server will reject the conflict with a clear
  // message if they actually submit).
  useEffect(() => {
    if (mode !== null) return;
    let cancelled = false;
    (async () => {
      if (!invite) {
        try {
          const me = await auth.me();
          if (!cancelled && me) {
            router.replace(next);
            return;
          }
        } catch {
          // 401 (no session) is the expected case here; fall through
          // to the setup/login decision below.
        }
      }
      try {
        const s = await auth.status();
        if (!cancelled) setMode(s.setup_needed ? "setup" : "login");
      } catch (e) {
        if (!cancelled) {
          setError(
            e instanceof ChimpFlixApiError
              ? `Server unreachable (${e.status})`
              : "Server unreachable",
          );
          setMode("login");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [mode, invite, next, router]);

  function setModeAndReset(next: Mode) {
    setMode(next);
    setError(null);
    setNotice(null);
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      if (mode === "setup") {
        await auth.setup({
          username: username.trim(),
          password,
          display_name: displayName.trim() || undefined,
          email: email.trim() || undefined,
        });
        router.push(next);
        router.refresh();
      } else if (mode === "register") {
        await auth.register({
          code: invite,
          username: username.trim(),
          password,
          display_name: displayName.trim() || undefined,
        });
        router.push(next);
        router.refresh();
      } else if (mode === "forgot") {
        await auth.requestPasswordReset(email.trim());
        // Same response shown whether or not the email is registered —
        // matches the server's no-enumeration response shape.
        setNotice(
          "If that email is registered, a reset link has been sent. Check your inbox (and spam folder).",
        );
        setEmail("");
      } else if (mode === "two_factor") {
        const trimmedCode = totpCode.trim();
        const body = useRecovery
          ? { challenge, recovery_code: trimmedCode }
          : { challenge, code: trimmedCode };
        await auth.loginTwoFactor(body);
        router.push(next);
        router.refresh();
      } else {
        const result = await auth.login({
          username: username.trim(),
          password,
        });
        if (result.status === "2fa_required") {
          // Don't clear password — useful if user wants to back out
          // and retry. Switch to the two-factor input form.
          setChallenge(result.challenge);
          setTotpCode("");
          setUseRecovery(false);
          setMode("two_factor");
        } else {
          router.push(next);
          router.refresh();
        }
      }
    } catch (e) {
      setError(parseError(e));
    } finally {
      setBusy(false);
    }
  }

  if (mode === null) {
    return (
      <main className="flex min-h-dvh items-center justify-center bg-background text-white">
        <div className="text-white/50">Loading…</div>
      </main>
    );
  }

  return (
    <main className="flex min-h-dvh items-center justify-center bg-background px-4 text-white">
      <div className="w-full max-w-sm">
        <h1 className="mb-2 text-3xl font-black tracking-tight text-(--color-accent)">
          {brandNameUpper()}
        </h1>
        <h2 className="mb-6 text-lg text-white/80">
          {mode === "setup"
            ? "Create the owner account"
            : mode === "register"
              ? "Create your account"
              : mode === "forgot"
                ? "Reset your password"
                : mode === "two_factor"
                  ? "Two-factor authentication"
                  : "Sign in to continue"}
        </h2>

        <form className="space-y-3" onSubmit={onSubmit}>
          {mode === "two_factor" ? (
            <>
              <label className="block">
                <span className="mb-1 block text-sm text-white/70">
                  {useRecovery ? "Recovery code" : "Authenticator code"}
                </span>
                <input
                  type="text"
                  name={useRecovery ? "recovery-code" : "otp"}
                  inputMode={useRecovery ? "text" : "numeric"}
                  autoComplete="one-time-code"
                  autoFocus
                  required
                  value={totpCode}
                  onChange={(e) => setTotpCode(e.target.value)}
                  placeholder={useRecovery ? "xxxx-xxxx-xxxx-xxxx" : "123456"}
                  // 64 covers both 6-digit TOTPs and the longest
                  // single-use recovery codes (8 groups of 4 chars
                  // joined by dashes = 39 chars, plus slack for any
                  // future format change).
                  maxLength={64}
                  className="w-full rounded bg-white/10 px-3 py-3 text-center text-lg font-mono tracking-widest outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                />
                <span className="mt-1 block text-xs text-white/40">
                  {useRecovery
                    ? "Enter one of your single-use recovery codes."
                    : "Enter the 6-digit code from your authenticator app."}
                </span>
              </label>
            </>
          ) : mode === "forgot" ? (
            <label className="block">
              <span className="mb-1 block text-sm text-white/70">Email</span>
              <input
                type="email"
                name="email"
                autoComplete="email"
                required
                autoFocus
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                maxLength={320}
                className="w-full rounded bg-white/10 px-3 py-2 outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
              <span className="mt-1 block text-xs text-white/40">
                Enter the email associated with your account.
              </span>
            </label>
          ) : (
            <>
              <label className="block">
                <span className="mb-1 block text-sm text-white/70">Username</span>
                <input
                  type="text"
                  name="username"
                  autoComplete="username"
                  required
                  autoFocus
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  maxLength={64}
                  className="w-full rounded bg-white/10 px-3 py-2.5 text-base outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                />
              </label>

              {(mode === "setup" || mode === "register") && (
                <label className="block">
                  <span className="mb-1 block text-sm text-white/70">
                    Display name <span className="text-white/40">(optional)</span>
                  </span>
                  <input
                    type="text"
                    name="display_name"
                    autoComplete="name"
                    value={displayName}
                    onChange={(e) => setDisplayName(e.target.value)}
                    maxLength={64}
                    className="w-full rounded bg-white/10 px-3 py-2.5 text-base outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                  />
                </label>
              )}

              {mode === "setup" && (
                <label className="block">
                  <span className="mb-1 block text-sm text-white/70">
                    Email <span className="text-white/40">(recommended)</span>
                  </span>
                  <input
                    type="email"
                    name="email"
                    autoComplete="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    maxLength={320}
                    className="w-full rounded bg-white/10 px-3 py-2.5 text-base outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                  />
                  <span className="mt-1 block text-xs text-white/40">
                    Required for self-service password reset.
                  </span>
                </label>
              )}

              <label className="block">
                <span className="mb-1 block text-sm text-white/70">Password</span>
                <input
                  type="password"
                  name="password"
                  autoComplete={
                    mode === "setup" || mode === "register"
                      ? "new-password"
                      : "current-password"
                  }
                  required
                  minLength={8}
                  maxLength={1024}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full rounded bg-white/10 px-3 py-2.5 text-base outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
                />
              </label>
            </>
          )}

          {error && (
            <div className="rounded bg-red-500/10 px-3 py-2 text-sm text-red-300 ring-1 ring-red-500/30">
              {error}
            </div>
          )}
          {notice && (
            <div className="rounded bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200 ring-1 ring-emerald-500/30">
              {notice}
            </div>
          )}

          <button
            type="submit"
            disabled={busy}
            className="w-full rounded bg-(--color-accent) px-3 py-3 text-base font-semibold text-white transition disabled:opacity-50"
          >
            {busy
              ? "…"
              : mode === "setup" || mode === "register"
                ? "Create account"
                : mode === "forgot"
                  ? "Send reset link"
                  : mode === "two_factor"
                    ? "Verify"
                    : "Sign in"}
          </button>
        </form>

        {/* Plex sign-in. Available for login + invite-bearing signup;
            hidden on setup / forgot / 2FA challenge where it doesn't
            apply. The invite-signup path passes the invite code along
            so the same flow that creates a user via the form can also
            create one via Plex OAuth. */}
        {(mode === "login" || mode === "register") && (
          <div className="mt-4 space-y-2">
            <div className="flex items-center gap-3 text-[10px] uppercase tracking-wider text-white/40">
              <span className="h-px flex-1 bg-white/10" />
              or
              <span className="h-px flex-1 bg-white/10" />
            </div>
            <PlexSignInButton
              intent={
                mode === "register"
                  ? { intent: "signup", invite_code: invite }
                  : { intent: "login" }
              }
              onSuccess={() => {
                router.push(next);
                router.refresh();
              }}
              onNotLinked={(plexUsername) =>
                setError(
                  `No ChimpFlix account is linked to "${plexUsername}". Ask the server owner for an invite, then link your Plex account from Settings → Account.`,
                )
              }
              onError={(msg) => setError(msg)}
              disabled={busy || (mode === "register" && !invite)}
            />
          </div>
        )}

        {/* Mode switchers. Hidden in setup mode (only one path on first run). */}
        {mode === "login" && (
          <div className="mt-4 text-center text-xs text-white/50">
            <button
              type="button"
              onClick={() => setModeAndReset("forgot")}
              className="text-white/70 underline-offset-2 hover:text-white hover:underline"
            >
              Forgot password?
            </button>
          </div>
        )}
        {mode === "forgot" && (
          <div className="mt-4 text-center text-xs text-white/50">
            <button
              type="button"
              onClick={() => setModeAndReset("login")}
              className="text-white/70 underline-offset-2 hover:text-white hover:underline"
            >
              ← Back to sign in
            </button>
          </div>
        )}
        {mode === "two_factor" && (
          <div className="mt-4 flex flex-col items-center gap-2 text-center text-xs text-white/50">
            <button
              type="button"
              onClick={() => {
                setUseRecovery((v) => !v);
                setTotpCode("");
                setError(null);
              }}
              className="text-white/70 underline-offset-2 hover:text-white hover:underline"
            >
              {useRecovery
                ? "Use authenticator code instead"
                : "Use a recovery code instead"}
            </button>
            <button
              type="button"
              onClick={() => {
                setChallenge("");
                setTotpCode("");
                setUseRecovery(false);
                setModeAndReset("login");
              }}
              className="text-white/70 underline-offset-2 hover:text-white hover:underline"
            >
              ← Back to sign in
            </button>
          </div>
        )}
      </div>
    </main>
  );
}

/// Parse the server's `{ error: { code, message } }` JSON error shape
/// and fall back gracefully if the response wasn't JSON.
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
      // Fall through to status code.
    }
    if (e.status === 429) return "Too many attempts. Try again in a moment.";
    if (e.status === 401 || e.status === 400) return "Invalid credentials.";
    return `Error ${e.status}`;
  }
  return "Network error";
}

