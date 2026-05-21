import { notFound } from "next/navigation";
import Link from "next/link";
import { LibraryBrowseClient } from "@/components/LibraryBrowseClient";
import { ModalRoot } from "@/components/ModalRoot";
import {
  items as itemsApi,
  libraries as librariesApi,
  type ItemKind,
  type ItemSort,
  type Library,
} from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";

const PAGE_SIZE = 60;
const VALID_SORTS: ReadonlyArray<ItemSort> = [
  "recently_added",
  "title",
  "year_desc",
  "year_asc",
  "rating_desc",
];

function itemKindFor(lib: Library): ItemKind {
  return lib.kind === "movies" ? "movie" : "show";
}

/// Power-user "see everything in this library" grid. The main
/// `/library/[id]` page keeps the Netflix-style hero + rails for
/// discovery; this page is the inventory view for finding a
/// specific title, browsing by sort order, or fixing the metadata
/// of files the parser couldn't auto-match.
///
/// Filters: kind chip (all/auto-matched/unmatched), sort dropdown,
/// search box. URL-driven so the back button works and links are
/// shareable.
export default async function LibraryBrowsePage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { id: idStr } = await params;
  const id = Number(idStr);
  if (!Number.isFinite(id) || id <= 0) notFound();

  await requireUser(`/library/${id}/browse`);

  const { libraries } = await librariesApi.list();
  const lib = libraries.find((l) => l.id === id);
  if (!lib) notFound();

  const sp = await searchParams;
  const rawSort = typeof sp.sort === "string" ? sp.sort : "recently_added";
  const sort: ItemSort = (VALID_SORTS as readonly string[]).includes(rawSort)
    ? (rawSort as ItemSort)
    : "recently_added";
  const q = typeof sp.q === "string" ? sp.q.trim() : "";
  const filterRaw = typeof sp.filter === "string" ? sp.filter : "all";
  const filter: "all" | "matched" | "unmatched" =
    filterRaw === "matched" || filterRaw === "unmatched" ? filterRaw : "all";
  const page = Math.max(1, Number(sp.page) || 1);

  const kind = itemKindFor(lib);

  const initial = await itemsApi.list({
    library_id: lib.id,
    kind,
    sort,
    page,
    page_size: PAGE_SIZE,
    q: q || undefined,
    auto_matched:
      filter === "matched" ? true : filter === "unmatched" ? false : undefined,
  });

  return (
    <main className="relative min-h-screen bg-background pb-24 pt-24 sm:pt-28">
      <div className="px-4 sm:px-8 md:px-12">
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <div className="text-[0.7rem] font-semibold uppercase tracking-[0.18em] text-accent">
              Library
            </div>
            <h1 className="mt-1 text-3xl font-bold tracking-tight">
              {lib.name}
            </h1>
            <p className="mt-1 text-sm text-white/55">
              {initial.total.toLocaleString()}{" "}
              {initial.total === 1 ? "title" : "titles"} · grid view
            </p>
          </div>
          <Link
            href={`/library/${lib.id}`}
            className="rounded border border-white/15 px-3 py-1.5 text-xs text-white/80 transition-colors hover:bg-white/5"
          >
            ← Back to library
          </Link>
        </div>

        <LibraryBrowseClient
          libraryId={lib.id}
          initialItems={initial.items}
          initialTotal={initial.total}
          initialPage={page}
          initialSort={sort}
          initialQuery={q}
          initialFilter={filter}
          pageSize={PAGE_SIZE}
          kind={kind}
        />
      </div>
      <ModalRoot />
    </main>
  );
}
