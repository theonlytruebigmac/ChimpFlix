import Link from "next/link";
import { brandName, brandNameUpper } from "@/lib/env";

// Rendered when an early server-side Plex call fails (network down,
// DNS, TLS, etc.) so the page chrome stays usable instead of crashing
// to the global error boundary. Gives the user actionable next steps.
export function ServerUnreachable({
  error,
  serverUrl,
}: {
  error: unknown;
  serverUrl?: string;
}) {
  const message = error instanceof Error ? error.message : String(error);
  return (
    <main className="flex min-h-screen flex-col items-center justify-center bg-black px-6 text-center text-white">
      <div className="select-none text-3xl font-black tracking-tight text-(--color-accent)">
        {brandNameUpper()}
      </div>
      <h1 className="mt-10 text-3xl font-bold">
        Couldn&apos;t reach your Plex server
      </h1>
      <p className="mt-3 max-w-lg text-white/65">
        {brandName()} tried to load your library but the server didn&apos;t
        answer. It might be offline, on a network this app can&apos;t
        reach, or temporarily down.
      </p>
      {serverUrl && (
        <p className="mt-2 font-mono text-xs text-white/40">{serverUrl}</p>
      )}
      <p className="mt-2 font-mono text-xs text-white/40">{message}</p>
      <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
        <Link
          href="/"
          className="rounded-md bg-(--color-accent) px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-(--color-accent-hover)"
        >
          Retry
        </Link>
        {/*
          Form POST (not Link) so the active-server cookie gets cleared
          before navigation. Without this the "Pick a different server"
          link would just bounce back to /select-server which redirects
          home because the (broken) server cookie is still set.
        */}
        <form action="/api/auth/clear-server" method="post">
          <button
            type="submit"
            className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
          >
            Pick a different server
          </button>
        </form>
        <form action="/api/auth/logout" method="post">
          <button
            type="submit"
            className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
          >
            Sign out
          </button>
        </form>
      </div>
    </main>
  );
}
