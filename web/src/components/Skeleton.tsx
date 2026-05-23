// Loading placeholders rendered inside Suspense boundaries while server
// components await Plex data. Match the geometry of the real component so
// the page doesn't reflow when content arrives.

export function CardSkeleton() {
  // Width mirrors `Card` exactly: ~2 cards across a 360px mobile viewport,
  // upscaling to the original 18rem at md+. Mismatched widths produce a
  // hydration warning (real grid uses w-44 / sm:w-56 / md:w-72) and a
  // visible layout shift when the placeholder swaps for the real card.
  return (
    <div className="w-44 flex-none sm:w-56 md:w-72">
      <div className="aspect-video w-full overflow-hidden rounded-md bg-white/5">
        <div className="h-full w-full animate-pulse bg-linear-to-br from-white/4 to-white/8" />
      </div>
    </div>
  );
}

export function RailSkeleton({ title }: { title?: string }) {
  return (
    <section className="px-4 sm:px-8 md:px-12 pb-1 pt-1">
      <h2 className="mb-3 text-[1.4rem] font-semibold tracking-tight">
        {title ?? <span className="inline-block h-6 w-48 rounded bg-white/10" />}
      </h2>
      <ul className="-mx-1 flex gap-1.5 overflow-hidden px-1 pb-12 pt-2">
        {Array.from({ length: 8 }).map((_, i) => (
          <li key={i} className="flex-none">
            <CardSkeleton />
          </li>
        ))}
      </ul>
    </section>
  );
}

export function HeroSkeleton() {
  // Visibility note: previous pulse used `from-white/4 via-white/2` which
  // is so close to the background colour that the whole hero zone reads
  // as a blank black rectangle on first paint. Users reported it looking
  // like nothing was loading. Bumped to white/8 with `animate-pulse` on
  // the title/description boxes so there's clearly *something* there
  // while the data resolves.
  return (
    <section className="relative h-[70vh] min-h-120 w-full overflow-hidden bg-linear-to-r from-white/8 to-background">
      <div className="absolute inset-0 animate-pulse bg-linear-to-r from-background via-white/5 to-transparent" />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-80 bg-linear-to-t from-background via-background/70 to-transparent" />
      <div className="relative z-10 flex h-full max-w-2xl flex-col justify-end px-4 sm:px-8 md:px-12 pb-36">
        <div className="mb-5 h-14 w-2/3 animate-pulse rounded bg-white/15" />
        <div className="mb-2 h-4 w-full animate-pulse rounded bg-white/10" />
        <div className="mb-7 h-4 w-3/4 animate-pulse rounded bg-white/10" />
        <div className="flex gap-3">
          <div className="h-11 w-32 animate-pulse rounded bg-white/15" />
          <div className="h-11 w-32 animate-pulse rounded bg-white/10" />
        </div>
      </div>
    </section>
  );
}
