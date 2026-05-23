import Link from "next/link";
import { brandNameUpper } from "@/lib/env";

// Global 404 boundary — catches any `notFound()` thrown from a server
// component (collection/[id], person/[id], library/[id], watch/[ratingKey],
// etc.) without each route having to ship its own 404 page. Sits next to
// app/error.tsx and inherits the same brand chrome.
export default function NotFound() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background px-6 text-center text-white">
      <div className="select-none text-3xl font-black tracking-tight text-(--color-accent)">
        {brandNameUpper()}
      </div>
      <h1 className="mt-10 text-3xl font-bold">Not found</h1>
      <p className="mt-3 max-w-lg text-white/65">
        The page you were looking for doesn&apos;t exist — or you don&apos;t
        have access to it. The link may be outdated, the title may have
        been removed from your library, or the URL may be wrong.
      </p>
      <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
        <Link
          href="/"
          className="rounded-md bg-(--color-accent) px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-(--color-accent-hover)"
        >
          Go home
        </Link>
        <Link
          href="/search"
          className="rounded-md border border-white/30 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
        >
          Search
        </Link>
      </div>
    </div>
  );
}
