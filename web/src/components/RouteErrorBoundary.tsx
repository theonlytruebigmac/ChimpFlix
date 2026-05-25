"use client";

import Link from "next/link";
import { useEffect } from "react";
import { devError } from "@/lib/dev-log";

/// Shared per-route error boundary primitive. Each route segment that
/// wants a contextual fallback declares a tiny `error.tsx` that
/// forwards to this — the per-route copy says "back to the library"
/// or "back to search" while the visual chrome stays consistent.
///
/// The root `app/error.tsx` catches anything that escapes a leaf
/// boundary, so route-level `error.tsx` files aren't required for
/// correctness — they exist so the global TopNav + chrome stay
/// mounted (Next.js unmounts only the failing segment) and so users
/// get a recovery link tailored to where they were.
export function RouteErrorBoundary({
  error,
  reset,
  title,
  fallbackHref = "/",
  fallbackLabel = "Back to home",
}: {
  error: Error & { digest?: string };
  reset: () => void;
  /// Heading shown in the boundary. Keep it short — one line, no
  /// trailing punctuation. Example: "Library couldn't load".
  title: string;
  /// Where the "back" button takes the user. Defaults to the home
  /// page; pass the parent route when the error happened in a leaf
  /// (e.g. /collection/[id] → "/" because there's no collection
  /// index page, but /settings/admin/users → "/settings/admin").
  fallbackHref?: string;
  fallbackLabel?: string;
}) {
  useEffect(() => {
    devError(`[${title}] render error:`, error);
  }, [error, title]);
  return (
    <main className="px-4 py-12 text-white">
      <div className="mx-auto max-w-xl rounded-lg border border-red-500/30 bg-red-500/5 p-6">
        <h1 className="text-xl font-semibold">{title}</h1>
        <p className="mt-2 text-sm text-white/65">
          {error.message ||
            "A request failed while rendering this page. The details are masked in production builds — check the server logs for the full stack."}
        </p>
        {error.digest && (
          <p className="mt-2 font-mono text-xs text-white/40">
            digest: {error.digest}
          </p>
        )}
        <div className="mt-5 flex flex-wrap items-center gap-3">
          <button
            type="button"
            onClick={reset}
            className="rounded-md bg-(--color-accent) px-4 py-2 text-sm font-semibold text-white hover:opacity-90"
          >
            Try again
          </button>
          <Link
            href={fallbackHref}
            className="rounded-md border border-white/30 px-4 py-2 text-sm text-white hover:border-white"
          >
            {fallbackLabel}
          </Link>
        </div>
      </div>
    </main>
  );
}
