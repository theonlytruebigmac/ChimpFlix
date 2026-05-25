"use client";

import Link from "next/link";
import { useEffect } from "react";
import { devError } from "@/lib/dev-log";

/// Settings-scoped error boundary. Sits under the settings tabs so a
/// failed admin endpoint doesn't blank the whole app — the chrome
/// stays, the panel shows recoverable copy + a retry button.
export default function SettingsError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    devError("[settings] render error:", error);
  }, [error]);
  return (
    <main className="px-4 py-10 text-white">
      <div className="mx-auto max-w-2xl rounded-lg border border-red-500/30 bg-red-500/5 p-6">
        <h1 className="text-xl font-semibold">This settings page couldn&apos;t load</h1>
        <p className="mt-2 text-sm text-white/65">
          {error.message ||
            "A request to the server failed while rendering this panel."}
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
            Retry
          </button>
          <Link
            href="/settings"
            className="rounded-md border border-white/30 px-4 py-2 text-sm text-white hover:border-white"
          >
            Settings home
          </Link>
        </div>
      </div>
    </main>
  );
}
