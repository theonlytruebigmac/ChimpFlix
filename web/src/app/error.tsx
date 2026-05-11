"use client";

import { useEffect } from "react";
import Link from "next/link";
import { Brand } from "@/components/Brand";

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    console.error("ChimpFlix render error:", error);
  }, [error]);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-black px-6 text-center text-white">
      <Brand size="lg" />
      <h1 className="mt-10 text-3xl font-bold">Something broke</h1>
      <p className="mt-3 max-w-lg text-white/65">
        {error.message ||
          "An error occurred while rendering this page. Check the server logs for the full stack."}
      </p>
      {error.digest && (
        <p className="mt-2 font-mono text-xs text-white/40">digest: {error.digest}</p>
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
          href="/"
          className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
        >
          Go home
        </Link>
      </div>
    </div>
  );
}
