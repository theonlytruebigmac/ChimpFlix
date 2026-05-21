import Link from "next/link";
import type { Collection } from "@/lib/chimpflix-api";
import { plexImage, plexSrcSet } from "@/lib/image";

/// Home rail for both auto (TMDB franchise) and manual (admin-curated)
/// collections. Tiles use the collection's own backdrop/poster art when
/// present; manual collections without an explicit poster get a
/// gradient name-only fallback (admins can upload art later via the
/// /settings/admin/library/collections detail panel).
///
/// Click → /collection/[id] (full page, not modal). Collections aren't a
/// MediaItem so they don't reuse `<Card>` — that one fires the modal
/// machinery which expects an item id.
export function CollectionsRail({ collections }: { collections: Collection[] }) {
  if (collections.length === 0) return null;
  return (
    <section
      className="zf-rise-in px-4 pb-1 pt-1 sm:px-8 md:px-12"
      style={{
        contentVisibility: "auto",
        containIntrinsicSize: "260px",
      }}
    >
      <h2 className="mb-3 text-lg font-semibold tracking-tight sm:text-xl md:text-[1.4rem]">
        Collections
      </h2>
      <ul className="-mx-1 flex snap-x snap-mandatory gap-1.5 overflow-x-auto px-1 pb-12 pt-2 scrollbar-none md:snap-none [&::-webkit-scrollbar]:hidden">
        {collections.map((c) => (
          <li key={c.id} className="snap-start flex-none">
            <CollectionCard collection={c} />
          </li>
        ))}
      </ul>
    </section>
  );
}

function CollectionCard({ collection }: { collection: Collection }) {
  const artPath = collection.backdrop_path ?? collection.poster_path;
  const img = plexImage(artPath ?? undefined, 480, 270);
  const srcSet = plexSrcSet(artPath ?? undefined, 480, 270);

  return (
    <div className="group relative w-44 flex-none sm:w-56 md:w-72 hover:z-50">
      <div className="card-scaler origin-center transition-transform duration-200 ease-out delay-200 group-hover:scale-110">
        <div className="overflow-hidden rounded-md bg-(--color-surface) shadow-md group-hover:shadow-2xl">
          <Link
            href={`/collection/${collection.id}`}
            aria-label={collection.name}
            className="block text-left"
          >
            <div className="relative aspect-video bg-gradient-to-br from-neutral-800 via-neutral-900 to-black">
              {img ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={img}
                  srcSet={srcSet}
                  alt=""
                  loading="lazy"
                  decoding="async"
                  className="h-full w-full object-cover"
                />
              ) : (
                /* Name-only fallback for collections without art.
                   Manual collections start here until an admin uploads
                   posters / sets a poster_item_id. */
                <div className="flex h-full w-full items-center justify-center px-3 text-center">
                  <span className="line-clamp-3 text-base font-semibold leading-tight text-white/85 drop-shadow">
                    {collection.name}
                  </span>
                </div>
              )}
              {/* Title overlay over art. Mirrors Card's treatment so the
                  rail reads consistently when art is present. Skipped
                  on the fallback because the name already dominates. */}
              {img && (
                <div className="pointer-events-none absolute inset-x-0 top-0 bg-linear-to-b from-black/85 via-black/40 to-transparent pb-10">
                  <div className="line-clamp-2 px-3 pt-2.5 text-sm font-semibold leading-tight drop-shadow-lg">
                    {collection.name}
                  </div>
                </div>
              )}
              {/* Auto badge in the corner so the user knows whether a
                  given collection is system-managed (TMDB) or admin-
                  authored. Manual collections render unbadged. */}
              {collection.kind === "auto" && (
                <div className="pointer-events-none absolute bottom-2 right-2 select-none rounded bg-black/60 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-white/80">
                  Collection
                </div>
              )}
            </div>
          </Link>
          <div className="flex items-baseline justify-between gap-2 px-3 py-2 text-xs text-white/55">
            <span className="truncate">{collection.item_count} title{collection.item_count === 1 ? "" : "s"}</span>
          </div>
        </div>
      </div>
    </div>
  );
}
