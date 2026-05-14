"use client";

import { Suspense, useEffect, useRef, useState } from "react";
import { useSearchParams } from "next/navigation";
import { brandNameUpper } from "@/lib/env";

type StartResponse = {
  pinId: number;
  code: string;
  linkUrl: string;
  authUrl: string;
  expiresAt: string;
};

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginContent />
    </Suspense>
  );
}

function LoginContent() {
  const searchParams = useSearchParams();
  const fromPlex = searchParams.get("from_plex") === "1";

  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<
    "idle" | "redirecting" | "polling" | "expired"
  >(fromPlex ? "polling" : "idle");
  const pollTimer = useRef<number | null>(null);
  const startedRef = useRef(false);

  // Begin OAuth: ask the server for a PIN + Plex auth URL, then redirect
  // the browser to plex.tv. After the user authorizes (usually one click
  // since they're already signed in at plex.tv), Plex bounces them back
  // to /login?from_plex=1 and the polling effect below picks up the token.
  async function start() {
    setError(null);
    setStatus("redirecting");
    try {
      const res = await fetch("/api/auth/start", { method: "POST" });
      if (!res.ok) {
        setError(`Failed to start auth (${res.status})`);
        setStatus("idle");
        return;
      }
      const data = (await res.json()) as StartResponse;
      window.location.href = data.authUrl;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStatus("idle");
    }
  }

  // When Plex bounces us back, poll for the token. The pending PIN cookie
  // (set by /api/auth/start) is the link between the two halves of the
  // flow — /api/auth/poll uses it to know which PIN to ask Plex about.
  useEffect(() => {
    if (status !== "polling") return;
    if (startedRef.current) return;
    startedRef.current = true;
    let cancelled = false;
    let attempts = 0;
    const MAX_ATTEMPTS = 30; // ~60s at 2s intervals
    const tick = async () => {
      attempts++;
      try {
        const res = await fetch("/api/auth/poll", { method: "POST" });
        if (cancelled) return;
        if (res.status === 200) {
          window.location.href = "/select-server";
          return;
        }
        if (res.status === 202 && attempts < MAX_ATTEMPTS) {
          pollTimer.current = window.setTimeout(tick, 2000);
          return;
        }
        setStatus("expired");
        setError(
          attempts >= MAX_ATTEMPTS
            ? "Timed out waiting for Plex confirmation."
            : `Auth check failed (${res.status})`,
        );
      } catch (e) {
        if (cancelled) return;
        setStatus("expired");
        setError(e instanceof Error ? e.message : String(e));
      }
    };
    pollTimer.current = window.setTimeout(tick, 500);
    return () => {
      cancelled = true;
      if (pollTimer.current) window.clearTimeout(pollTimer.current);
    };
  }, [status]);

  return (
    <div className="relative min-h-screen overflow-hidden bg-black">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_top,rgba(70,12,16,0.55)_0%,rgba(0,0,0,0.95)_55%,#000_100%)]"
      />

      <header className="relative z-10">
        <div className="px-8 py-5 sm:px-12">
          <div className="select-none text-3xl font-black tracking-tight text-(--color-accent) sm:text-4xl">
            {brandNameUpper()}
          </div>
        </div>
      </header>

      <main className="relative z-10 mx-auto w-full max-w-md px-6 pt-20 sm:pt-28">
        {status === "polling" ? (
          <>
            <h1 className="mb-2 text-4xl font-bold leading-tight tracking-tight text-white sm:text-[2.75rem]">
              Signing you in…
            </h1>
            <p className="mb-10 text-base text-(--color-muted)">
              Hang tight — confirming with Plex.
            </p>
            <div className="flex items-center gap-3 text-(--color-muted)">
              <span className="inline-block h-2.5 w-2.5 animate-pulse rounded-full bg-(--color-accent)" />
              Waiting for confirmation…
            </div>
          </>
        ) : status === "expired" ? (
          <>
            <h1 className="mb-2 text-4xl font-bold leading-tight tracking-tight text-white sm:text-[2.75rem]">
              That didn&apos;t work
            </h1>
            <p className="mb-10 text-base text-(--color-muted)">
              {error ?? "Plex didn't confirm in time. Try again?"}
            </p>
            <button
              onClick={start}
              className="w-full rounded-md bg-(--color-accent) py-4 text-lg font-semibold text-white shadow-md transition-colors hover:bg-(--color-accent-hover)"
            >
              Sign in with Plex
            </button>
          </>
        ) : (
          <>
            <h1 className="mb-2 text-4xl font-bold leading-tight tracking-tight text-white sm:text-[2.75rem]">
              Sign in to continue
            </h1>
            <p className="mb-10 text-base text-(--color-muted)">
              Use your Plex account to unlock the library.
            </p>

            <button
              onClick={start}
              disabled={status === "redirecting"}
              className="w-full rounded-md bg-(--color-accent) py-4 text-lg font-semibold text-white shadow-md transition-colors hover:bg-(--color-accent-hover) disabled:opacity-60"
            >
              {status === "redirecting" ? "Redirecting to Plex…" : "Sign in with Plex"}
            </button>

            <p className="mt-8 text-sm text-(--color-muted)">
              No Plex account?{" "}
              <a
                href="https://www.plex.tv/sign-up/"
                target="_blank"
                rel="noreferrer"
                className="font-medium text-white hover:underline"
              >
                Create one.
              </a>
            </p>
          </>
        )}

        {error && status !== "expired" && (
          <p className="mt-6 text-sm text-(--color-accent)">{error}</p>
        )}
      </main>
    </div>
  );
}
