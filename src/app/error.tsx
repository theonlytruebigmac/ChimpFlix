"use client";

import Link from "next/link";
import { useEffect } from "react";
import { brandName, brandNameUpper } from "@/lib/env";

// Global error boundary — catches any uncaught error from a server
// component render or client effect under the root layout. Shows the
// digest (always available, even in prod) and the message (dev only,
// stripped in prod for safety) so we can diagnose without digging
// through container logs.
export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    // Mirror to the browser console for devtools-based diagnosis.
    // Production server logs still hold the full stack — this is the
    // client-side breadcrumb.
    console.error(`${brandName()} render error:`, error);
  }, [error]);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-black px-6 text-center text-white">
      <div className="select-none text-3xl font-black tracking-tight text-(--color-accent)">
        {brandNameUpper()}
      </div>
      <h1 className="mt-10 text-3xl font-bold">Something broke</h1>
      <p className="mt-3 max-w-lg text-white/65">
        {error.message ||
          "A server error occurred while rendering this page. The details are masked in production builds — check the server logs for the full stack."}
      </p>
      {error.digest && (
        <p className="mt-2 font-mono text-xs text-white/40">
          digest: {error.digest}
        </p>
      )}
      <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
        <button
          type="button"
          onClick={reset}
          className="rounded-md bg-(--color-accent) px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-(--color-accent-hover)"
        >
          Try again
        </button>
        <Link
          href="/select-server"
          className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
        >
          Switch server
        </Link>
        <form action="/api/auth/logout" method="post">
          <button
            type="submit"
            className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
          >
            Sign out
          </button>
        </form>
      </div>
    </div>
  );
}
