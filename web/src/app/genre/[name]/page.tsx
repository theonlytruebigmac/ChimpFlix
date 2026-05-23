import { GenreBrowseClient, type GenreKindFilter } from "@/components/GenreBrowseClient";
import { ModalRoot } from "@/components/ModalRoot";
import {
  items as itemsApi,
  type ItemSort,
} from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";

const DEFAULT_PAGE_SIZE = 60;
// Grid-friendly sizes (multiples of 6 to play well with up to 6-column
// rows). Matches the `pageSizes` prop on the GenreBrowseClient's
// Pagination component.
const ALLOWED_PAGE_SIZES = [24, 60, 120] as const;
const VALID_SORTS: ReadonlyArray<ItemSort> = [
  "recently_added",
  "title",
  "year_desc",
  "year_asc",
  "rating_desc",
  "duration_desc",
  "duration_asc",
  "last_played",
  "random",
  "size_desc",
  "size_asc",
];

export default async function GenrePage({
  params,
  searchParams,
}: {
  params: Promise<{ name: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { name } = await params;
  const sp = await searchParams;
  const genre = decodeURIComponent(name);
  await requireUser(`/genre/${name}`);

  const rawSort = typeof sp.sort === "string" ? sp.sort : "recently_added";
  const sort: ItemSort = (VALID_SORTS as readonly string[]).includes(rawSort)
    ? (rawSort as ItemSort)
    : "recently_added";
  const rawKind = typeof sp.type === "string" ? sp.type : undefined;
  const kind: GenreKindFilter =
    rawKind === "movie" || rawKind === "show" ? rawKind : "all";
  const page = Math.max(1, Number(sp.page) || 1);
  const requestedPageSize = Number(sp.page_size) || DEFAULT_PAGE_SIZE;
  const pageSize = (ALLOWED_PAGE_SIZES as readonly number[]).includes(
    requestedPageSize,
  )
    ? requestedPageSize
    : DEFAULT_PAGE_SIZE;

  // The random sort needs a deterministic seed so paging doesn't
  // reshuffle between requests. Same pattern as /library/[id]/browse:
  // client generates one when the user picks Shuffle and threads it
  // through ?seed=, deep-links without one fall back to 1.
  const rawSeed = typeof sp.seed === "string" ? Number(sp.seed) : NaN;
  const seed =
    sort === "random" && Number.isFinite(rawSeed) && rawSeed > 0
      ? Math.floor(rawSeed)
      : sort === "random"
        ? 1
        : null;

  // Server-side filter. `kind` defaults to undefined ("all kinds")
  // when the URL doesn't request one — previous behaviour. The
  // dropdown propagated via `?type=movie|show` now actually filters
  // the result set rather than being silently dropped.
  const initial = await itemsApi.list({
    genre,
    sort,
    page,
    page_size: pageSize,
    kind: kind === "all" ? undefined : kind,
    random_seed: seed ?? undefined,
  });

  return (
    <main className="relative min-h-screen bg-background">
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <div className="mb-4">
          <div className="text-[0.7rem] font-semibold uppercase tracking-[0.18em] text-accent">
            Genre
          </div>
          <h1 className="mt-1 text-4xl font-bold tracking-tight">{genre}</h1>
          <p className="mt-1 text-sm text-white/55">
            {initial.total.toLocaleString()}{" "}
            {initial.total === 1 ? "title" : "titles"}
          </p>
        </div>
        <GenreBrowseClient
          genreSegment={name}
          initialItems={initial.items}
          initialTotal={initial.total}
          initialPage={page}
          initialSort={sort}
          initialKind={kind}
          initialSeed={seed}
          pageSize={pageSize}
        />
      </div>
      <ModalRoot />
    </main>
  );
}
