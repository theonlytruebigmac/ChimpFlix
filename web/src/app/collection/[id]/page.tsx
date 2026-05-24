import { notFound } from "next/navigation";
import { CollectionBrowseClient } from "@/components/CollectionBrowseClient";
import { ModalRoot } from "@/components/ModalRoot";
import {
  ChimpFlixApiError,
  collections as collectionsApi,
} from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";

export default async function CollectionPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  await requireUser(`/collection/${id}`);
  const idNum = Number.parseInt(id, 10);
  if (!Number.isFinite(idNum) || idNum <= 0) notFound();

  let detail;
  try {
    detail = await collectionsApi.get(idNum);
  } catch (e) {
    if (e instanceof ChimpFlixApiError && e.status === 404) notFound();
    throw e;
  }

  const backdrop = detail.backdrop_path ?? detail.poster_path ?? null;

  return (
    <main className="relative min-h-screen bg-background">

      {/* Header banner with backdrop, name, and overview. Mirrors the
          modal hero treatment but at page scale. */}
      <section className="relative h-[40vh] min-h-72 w-full overflow-hidden">
        {backdrop && (
          // Collection page LCP — same eager + high-priority treatment
          // as the home Hero so the banner doesn't lag the title text.
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={backdrop}
            alt=""
            fetchPriority="high"
            decoding="async"
            className="zf-fade-in absolute inset-0 h-full w-full object-cover"
          />
        )}
        <div className="pointer-events-none absolute inset-0 bg-linear-to-r from-black/95 via-black/55 to-transparent" />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 h-72 bg-linear-to-t from-(--color-background) via-(--color-background)/70 to-transparent" />
        <div className="relative z-10 flex h-full max-w-2xl flex-col justify-end px-4 sm:px-8 md:px-12 pb-12 pt-24 sm:pt-28">
          <div className="mb-2 text-xs font-bold tracking-[0.35em] text-(--color-accent)">
            COLLECTION
          </div>
          <h1 className="mb-4 text-5xl font-black uppercase leading-[0.95] tracking-tight drop-shadow-lg">
            {detail.name}
          </h1>
          {detail.overview && (
            <p className="line-clamp-4 text-base leading-relaxed text-white/90 drop-shadow">
              {detail.overview}
            </p>
          )}
          <div className="mt-3 text-sm text-white/60">
            {detail.item_count} {detail.item_count === 1 ? "title" : "titles"}
          </div>
        </div>
      </section>

      <div className="relative z-20 px-4 sm:px-8 md:px-12 pb-24">
        <CollectionBrowseClient items={detail.items} />
      </div>
      <ModalRoot />
    </main>
  );
}
