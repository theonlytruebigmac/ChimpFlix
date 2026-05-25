"use client";

import Link from "next/link";
import { useEffect } from "react";
import { devError } from "@/lib/dev-log";

/// Playback-route-scoped error boundary. The player surface has its
/// own fatal-error handling for the actual `<video>` element — this
/// boundary only catches errors that escape the player tree itself
/// (a failed metadata fetch on first paint, a bad route param, etc.)
/// and steers the user back to a place they can recover from.
export default function WatchError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    devError("[watch] render error:", error);
  }, [error]);
  return (
    <main className="flex min-h-dvh flex-col items-center justify-center bg-black px-6 text-center text-white">
      <h1 className="text-2xl font-bold">Playback couldn&apos;t start</h1>
      <p className="mt-3 max-w-md text-sm text-white/65">
        Something broke while loading this title. Try again, or head
        back to the library to pick something else.
      </p>
      {error.digest && (
        <p className="mt-2 font-mono text-xs text-white/35">
          digest: {error.digest}
        </p>
      )}
      <div className="mt-6 flex flex-wrap items-center justify-center gap-3">
        <button
          type="button"
          onClick={reset}
          className="rounded-md bg-(--color-accent) px-5 py-2.5 text-sm font-semibold text-white hover:opacity-90"
        >
          Try again
        </button>
        <Link
          href="/"
          className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white hover:border-white"
        >
          Back to home
        </Link>
      </div>
    </main>
  );
}
