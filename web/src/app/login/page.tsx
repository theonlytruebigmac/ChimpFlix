"use client";

import { FormEvent, useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { ApiClientError, chimpflix } from "@/lib/api";
import { Brand } from "@/components/Brand";

type Mode = "loading" | "login" | "setup";

export default function LoginPage() {
  const router = useRouter();
  const [mode, setMode] = useState<Mode>("loading");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  // Decide what to render: first-run setup form, normal login, or
  // a banner if the server is unreachable.
  useEffect(() => {
    let cancelled = false;
    chimpflix.auth
      .status()
      .then((s) => {
        if (cancelled) return;
        setMode(s.setup_needed ? "setup" : "login");
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
        setMode("login");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      if (mode === "setup") {
        await chimpflix.auth.setup({ username, password });
      } else {
        await chimpflix.auth.login({ username, password });
      }
      router.replace("/");
    } catch (err) {
      if (err instanceof ApiClientError) {
        setError(err.message);
      } else {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="relative min-h-screen overflow-hidden bg-black">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_top,rgba(70,12,16,0.55)_0%,rgba(0,0,0,0.95)_55%,#000_100%)]"
      />

      <header className="relative z-10">
        <div className="px-8 py-5 sm:px-12">
          <Brand size="lg" />
        </div>
      </header>

      <main className="relative z-10 mx-auto w-full max-w-md px-6 pt-12 sm:pt-20">
        {mode === "loading" ? (
          <p className="text-white/55">Loading…</p>
        ) : mode === "setup" ? (
          <>
            <h1 className="mb-2 text-4xl font-bold leading-tight tracking-tight sm:text-[2.75rem]">
              First-run setup
            </h1>
            <p className="mb-10 text-base text-(--color-muted)">
              Create the owner account for this ChimpFlix server.
            </p>
          </>
        ) : (
          <>
            <h1 className="mb-2 text-4xl font-bold leading-tight tracking-tight sm:text-[2.75rem]">
              Sign in
            </h1>
            <p className="mb-10 text-base text-(--color-muted)">
              Access your ChimpFlix library.
            </p>
          </>
        )}

        {(mode === "login" || mode === "setup") && (
          <form onSubmit={onSubmit} className="space-y-4">
            <div>
              <label className="mb-2 block text-sm font-medium text-white/70">
                Username
              </label>
              <input
                type="text"
                autoComplete="username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                required
                className="block w-full rounded-md border border-white/15 bg-(--color-surface) px-3 py-3 text-white placeholder-white/40 focus:border-(--color-accent) focus:outline-none"
              />
            </div>
            <div>
              <label className="mb-2 block text-sm font-medium text-white/70">
                Password
              </label>
              <input
                type="password"
                autoComplete={
                  mode === "setup" ? "new-password" : "current-password"
                }
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                minLength={mode === "setup" ? 8 : undefined}
                className="block w-full rounded-md border border-white/15 bg-(--color-surface) px-3 py-3 text-white placeholder-white/40 focus:border-(--color-accent) focus:outline-none"
              />
              {mode === "setup" && (
                <p className="mt-1 text-xs text-white/45">
                  Minimum 8 characters.
                </p>
              )}
            </div>
            {error && <p className="text-sm text-(--color-accent)">{error}</p>}
            <button
              type="submit"
              disabled={submitting}
              className="w-full rounded-md bg-(--color-accent) py-3.5 text-lg font-semibold text-white shadow-md transition-colors hover:bg-(--color-accent-hover) disabled:opacity-60"
            >
              {submitting
                ? "…"
                : mode === "setup"
                  ? "Create owner"
                  : "Sign in"}
            </button>
          </form>
        )}
      </main>
    </div>
  );
}
