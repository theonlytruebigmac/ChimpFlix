// Loading placeholders rendered inside Suspense boundaries while server
// components await Plex data. Match the geometry of the real component so
// the page doesn't reflow when content arrives.

export function CardSkeleton() {
  return (
    <div className="w-72 flex-none">
      <div className="aspect-video w-full overflow-hidden rounded-md bg-white/5">
        <div className="h-full w-full animate-pulse bg-linear-to-br from-white/[0.04] to-white/[0.08]" />
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
  return (
    <section className="relative h-[70vh] min-h-120 w-full overflow-hidden bg-linear-to-r from-white/[0.04] to-(--color-background)">
      <div className="absolute inset-0 animate-pulse bg-linear-to-r from-(--color-background) via-white/[0.02] to-transparent" />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-80 bg-linear-to-t from-(--color-background) via-(--color-background)/70 to-transparent" />
      <div className="relative z-10 flex h-full max-w-2xl flex-col justify-end px-4 sm:px-8 md:px-12 pb-36">
        <div className="mb-5 h-14 w-2/3 rounded bg-white/10" />
        <div className="mb-2 h-4 w-full rounded bg-white/5" />
        <div className="mb-7 h-4 w-3/4 rounded bg-white/5" />
        <div className="flex gap-3">
          <div className="h-11 w-32 rounded bg-white/10" />
          <div className="h-11 w-32 rounded bg-white/5" />
        </div>
      </div>
    </section>
  );
}
