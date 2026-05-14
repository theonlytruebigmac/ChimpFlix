"use client";

import { Suspense, useEffect, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { brandNameUpper } from "@/lib/env";
import { auth, ChimpFlixApiError } from "@/lib/chimpflix-api";

type Mode = "login" | "setup" | "register";

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
  const next = search.get("next") || "/";
  const invite = search.get("invite")?.trim() ?? "";

  const [mode, setMode] = useState<Mode | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // First-run detection. Priority: invite link → register form, otherwise
  // the server's setup-needed flag chooses between setup vs login.
  useEffect(() => {
    if (invite) {
      setMode("register");
      return;
    }
    let cancelled = false;
    auth
      .status()
      .then((s) => {
        if (!cancelled) setMode(s.setup_needed ? "setup" : "login");
      })
      .catch((e) => {
        if (!cancelled) {
          setError(
            e instanceof ChimpFlixApiError
              ? `Server unreachable (${e.status})`
              : "Server unreachable",
          );
          setMode("login");
        }
      });
    return () => {
      cancelled = true;
    };
  }, [invite]);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      if (mode === "setup") {
        await auth.setup({
          username: username.trim(),
          password,
          display_name: displayName.trim() || undefined,
        });
      } else if (mode === "register") {
        await auth.register({
          code: invite,
          username: username.trim(),
          password,
          display_name: displayName.trim() || undefined,
        });
      } else {
        await auth.login({ username: username.trim(), password });
      }
      router.push(next);
      router.refresh();
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        try {
          const parsed = JSON.parse(e.body) as { error?: string };
          setError(parsed.error ?? `Error ${e.status}`);
        } catch {
          setError(`Error ${e.status}`);
        }
      } else {
        setError("Network error");
      }
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
              : "Sign in to continue"}
        </h2>

        <form className="space-y-3" onSubmit={onSubmit}>
          <label className="block">
            <span className="mb-1 block text-sm text-white/70">Username</span>
            <input
              type="text"
              autoComplete="username"
              required
              autoFocus
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              className="w-full rounded bg-white/10 px-3 py-2 outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            />
          </label>

          {(mode === "setup" || mode === "register") && (
            <label className="block">
              <span className="mb-1 block text-sm text-white/70">
                Display name <span className="text-white/40">(optional)</span>
              </span>
              <input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="w-full rounded bg-white/10 px-3 py-2 outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
              />
            </label>
          )}

          <label className="block">
            <span className="mb-1 block text-sm text-white/70">Password</span>
            <input
              type="password"
              autoComplete={
                mode === "setup" || mode === "register"
                  ? "new-password"
                  : "current-password"
              }
              required
              minLength={8}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full rounded bg-white/10 px-3 py-2 outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
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
            className="w-full rounded bg-(--color-accent) px-3 py-2 font-semibold text-white transition disabled:opacity-50"
          >
            {busy
              ? "…"
              : mode === "setup" || mode === "register"
                ? "Create account"
                : "Sign in"}
          </button>
        </form>
      </div>
    </main>
  );
}
