import { HistoryBrowseClient } from "@/components/HistoryBrowseClient";
import { ModalRoot } from "@/components/ModalRoot";
import { playState as playStateApi } from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";

const DEFAULT_PAGE_SIZE = 60;
// Grid-friendly sizes (multiples of 6). Matches the `pageSizes` prop on
// HistoryBrowseClient's Pagination component.
const ALLOWED_PAGE_SIZES = [24, 60, 120] as const;

export default async function HistoryPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  await requireUser("/history");
  const sp = await searchParams;
  const page = Math.max(1, Number(sp.page) || 1);
  const requestedPageSize = Number(sp.page_size) || DEFAULT_PAGE_SIZE;
  const pageSize = (ALLOWED_PAGE_SIZES as readonly number[]).includes(
    requestedPageSize,
  )
    ? requestedPageSize
    : DEFAULT_PAGE_SIZE;

  const { items, total } = await playStateApi.history({
    limit: pageSize,
    page,
  });

  return (
    <main className="relative min-h-screen bg-background">
      <div className="px-4 sm:px-8 md:px-12 pb-24 pt-24 sm:pt-28">
        <div className="mb-4">
          <h1 className="text-4xl font-bold tracking-tight">Watch History</h1>
          {total > 0 && (
            <p className="mt-1 text-sm text-white/55">
              {total.toLocaleString()}{" "}
              {total === 1 ? "title" : "titles"}
            </p>
          )}
        </div>
        <HistoryBrowseClient
          initialItems={items}
          initialTotal={total}
          initialPage={page}
          pageSize={pageSize}
        />
      </div>
      <ModalRoot />
    </main>
  );
}
