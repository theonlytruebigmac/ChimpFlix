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

const DEFAULT_PAGE_SIZE = 60;
// Grid-friendly sizes: 24 = 4 rows of 6 / 8 rows of 3, 60 = 10 rows of 6,
// 120 = 20 rows of 6. Matches the `pageSizes` prop on the shared
// Pagination component below.
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

const VALID_RESOLUTIONS: ReadonlySet<string> = new Set([
  "sd",
  "720",
  "1080",
  "4k",
]);
const VALID_HDR: ReadonlySet<string> = new Set(["sdr", "hdr10", "hlg"]);
const VALID_CODECS: ReadonlySet<string> = new Set([
  "hevc",
  "h264",
  "av1",
  "vp9",
  "mpeg4",
  "mpeg2video",
  "other",
]);

/// Read a CSV-encoded query-string param, trim/lowercase each token,
/// and drop anything not in `valid`. Returns `[]` (empty) when nothing
/// is set so callers can short-circuit cleanly.
function parseCsvWhitelisted(
  raw: string | string[] | undefined,
  valid: ReadonlySet<string>,
): string[] {
  if (typeof raw !== "string" || raw.trim().length === 0) return [];
  return raw
    .split(",")
    .map((t) => t.trim().toLowerCase())
    .filter((t) => valid.has(t));
}

type MatchFilter = "all" | "matched" | "unmatched";
type StatusFilter = "all" | "unwatched" | "in_progress" | "watched";
type DecadeFilter =
  | "all"
  | "2020"
  | "2010"
  | "2000"
  | "1990"
  | "1980"
  | "1970"
  | "older";

const VALID_STATUSES: ReadonlyArray<StatusFilter> = [
  "all",
  "unwatched",
  "in_progress",
  "watched",
];
const VALID_DECADES: ReadonlyArray<DecadeFilter> = [
  "all",
  "2020",
  "2010",
  "2000",
  "1990",
  "1980",
  "1970",
  "older",
];

function decadeRange(d: DecadeFilter): {
  year_min?: number;
  year_max?: number;
} {
  if (d === "all") return {};
  if (d === "older") return { year_max: 1969 };
  const start = Number(d);
  return { year_min: start, year_max: start + 9 };
}

function itemKindFor(lib: Library): ItemKind {
  return lib.kind === "movies" ? "movie" : "show";
}

/// Power-user "see everything in this library" grid. The main
/// `/library/[id]` page keeps the Netflix-style hero + rails for
/// discovery; this page is the inventory view for finding a
/// specific title, browsing by sort order, or fixing the metadata
/// of files the parser couldn't auto-match.
///
/// Filters: match-state chip (all/auto-matched/unmatched), per-user
/// status (unwatched/in-progress/watched), decade dropdown, sort
/// dropdown, search box. URL-driven so the back button works and
/// links are shareable.
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
  const filter: MatchFilter =
    filterRaw === "matched" || filterRaw === "unmatched" ? filterRaw : "all";
  const statusRaw = typeof sp.status === "string" ? sp.status : "all";
  const status: StatusFilter = (VALID_STATUSES as readonly string[]).includes(
    statusRaw,
  )
    ? (statusRaw as StatusFilter)
    : "all";
  const decadeRaw = typeof sp.decade === "string" ? sp.decade : "all";
  const decade: DecadeFilter = (VALID_DECADES as readonly string[]).includes(
    decadeRaw,
  )
    ? (decadeRaw as DecadeFilter)
    : "all";
  const page = Math.max(1, Number(sp.page) || 1);
  const requestedPageSize = Number(sp.page_size) || DEFAULT_PAGE_SIZE;
  const pageSize = (ALLOWED_PAGE_SIZES as readonly number[]).includes(
    requestedPageSize,
  )
    ? requestedPageSize
    : DEFAULT_PAGE_SIZE;

  // The random sort needs a deterministic seed so paging doesn't
  // reshuffle between requests. Client normally generates one when the
  // user picks "Shuffle" and threads it through the URL; if the URL
  // arrives without one (deep-link, bookmark, manual entry), fall back
  // to 1 so the server has *something* — the client will write a real
  // seed the next time the user touches the sort/reshuffle UI.
  const rawSeed = typeof sp.seed === "string" ? Number(sp.seed) : NaN;
  const seed =
    sort === "random" && Number.isFinite(rawSeed) && rawSeed > 0
      ? Math.floor(rawSeed)
      : sort === "random"
        ? 1
        : null;

  const yearRange = decadeRange(decade);

  const resolutions = parseCsvWhitelisted(sp.resolutions, VALID_RESOLUTIONS);
  const hdr = parseCsvWhitelisted(sp.hdr, VALID_HDR);
  const codecs = parseCsvWhitelisted(sp.codecs, VALID_CODECS);

  const kind = itemKindFor(lib);

  const initial = await itemsApi.list({
    library_id: lib.id,
    kind,
    sort,
    page,
    page_size: pageSize,
    q: q || undefined,
    auto_matched:
      filter === "matched" ? true : filter === "unmatched" ? false : undefined,
    unwatched_only: status === "unwatched" ? true : undefined,
    in_progress_only: status === "in_progress" ? true : undefined,
    watched_only: status === "watched" ? true : undefined,
    year_min: yearRange.year_min,
    year_max: yearRange.year_max,
    random_seed: seed ?? undefined,
    resolutions: resolutions.length > 0 ? resolutions : undefined,
    hdr: hdr.length > 0 ? hdr : undefined,
    codecs: codecs.length > 0 ? codecs : undefined,
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
          initialStatus={status}
          initialDecade={decade}
          initialSeed={seed}
          initialResolutions={resolutions}
          initialHdr={hdr}
          initialCodecs={codecs}
          pageSize={pageSize}
          kind={kind}
        />
      </div>
      <ModalRoot />
    </main>
  );
}
