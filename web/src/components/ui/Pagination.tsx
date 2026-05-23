"use client";

/// Shared pagination footer used by both admin tables and user-facing
/// browse grids. Pairs a page-size selector with a compact
/// page-number row (first, last, current, and the two on either
/// side; ellipses fill the gap).
///
/// The whole row is rendered server-component-safe (no client-state
/// of its own) — page + page size are owned by the caller, so each
/// table can decide whether to drive them from URL query params,
/// local state, or somewhere else.
///
/// Callers:
///   • Library → Scheduled Tasks → Job queue
///   • Library → Scheduled Tasks → Activity (recent runs)
///   • Maintenance → Logs → Audit trail
///   • /library/[id]/browse grid
///   • /genre/[name]
///   • /collection/[id]
///   • /history

import type { ReactNode } from "react";

const DEFAULT_PAGE_SIZES = [10, 25, 50, 100] as const;
export const DEFAULT_PAGE_SIZE = 25;

interface Props {
  /// 1-based current page.
  page: number;
  pageSize: number;
  /// Total number of rows across all pages — drives the page count
  /// AND the "X of Y" summary. Pass `null` when the API doesn't
  /// return a total (cursor-paginated surfaces); we degrade to a
  /// Prev/Next-only control without a final-page link.
  total: number | null;
  onPageChange: (next: number) => void;
  onPageSizeChange: (next: number) => void;
  /// Optional custom size choices. Defaults to 10 / 25 / 50 / 100.
  pageSizes?: ReadonlyArray<number>;
  /// Inline label for the row count, e.g. "jobs", "runs", "entries".
  /// Used in the "X of Y jobs" string. Defaults to "rows."
  noun?: string;
  /// Optional left-side slot for extra controls (refresh button,
  /// filters summary, etc.).
  leading?: ReactNode;
}

export function Pagination({
  page,
  pageSize,
  total,
  onPageChange,
  onPageSizeChange,
  pageSizes = DEFAULT_PAGE_SIZES,
  noun = "rows",
  leading,
}: Props) {
  const totalPages = total == null
    ? null
    : Math.max(1, Math.ceil(total / pageSize));
  const start = total == null || total === 0 ? 0 : (page - 1) * pageSize + 1;
  const end =
    total == null
      ? page * pageSize
      : Math.min(page * pageSize, total);

  return (
    <div className="flex flex-wrap items-center justify-between gap-3 pt-4 text-[12px] text-white/65">
      <div className="flex flex-wrap items-center gap-3">
        {leading}
        <label className="flex items-center gap-2">
          <span>Show</span>
          <select
            value={pageSize}
            onChange={(e) => onPageSizeChange(Number(e.target.value))}
            className="rounded-md border border-white/15 bg-black/40 px-2 py-1 text-[12.5px] text-white/90 focus:border-white/35 focus:outline-none"
          >
            {pageSizes.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          <span>per page</span>
        </label>
        {total != null && (
          <span className="text-white/45">
            {total === 0
              ? `No ${noun}`
              : `${start.toLocaleString()}–${end.toLocaleString()} of ${total.toLocaleString()} ${noun}`}
          </span>
        )}
      </div>
      <PagerButtons
        page={page}
        totalPages={totalPages}
        onPageChange={onPageChange}
      />
    </div>
  );
}

function PagerButtons({
  page,
  totalPages,
  onPageChange,
}: {
  page: number;
  totalPages: number | null;
  onPageChange: (next: number) => void;
}) {
  // Unknown-total case (cursor-paginated APIs): Prev/Next only.
  if (totalPages == null) {
    return (
      <div className="flex items-center gap-1">
        <PageButton
          disabled={page <= 1}
          onClick={() => onPageChange(page - 1)}
        >
          ‹ Prev
        </PageButton>
        <span className="px-2 text-[12px] text-white/45">page {page}</span>
        <PageButton onClick={() => onPageChange(page + 1)}>Next ›</PageButton>
      </div>
    );
  }

  if (totalPages <= 1) {
    return null;
  }

  return (
    <nav
      aria-label="Pagination"
      className="flex flex-wrap items-center gap-1"
    >
      <PageButton
        disabled={page <= 1}
        onClick={() => onPageChange(page - 1)}
      >
        ‹ Prev
      </PageButton>
      {visiblePages(page, totalPages).map((p, i) =>
        p === "…" ? (
          <span
            key={`gap-${i}`}
            aria-hidden
            className="px-1.5 text-[12px] text-white/40"
          >
            …
          </span>
        ) : (
          <PageButton
            key={p}
            onClick={() => onPageChange(p)}
            active={p === page}
            ariaCurrent={p === page ? "page" : undefined}
          >
            {p}
          </PageButton>
        ),
      )}
      <PageButton
        disabled={page >= totalPages}
        onClick={() => onPageChange(page + 1)}
      >
        Next ›
      </PageButton>
    </nav>
  );
}

/// Compact "1 … 4 5 6 … 20" layout. Small ranges (≤7) render every
/// page so there's no jarring "… 1 2 3 …" with a tiny pool.
function visiblePages(current: number, total: number): Array<number | "…"> {
  if (total <= 7) return Array.from({ length: total }, (_, i) => i + 1);
  const out: Array<number | "…"> = [1];
  const start = Math.max(2, current - 2);
  const end = Math.min(total - 1, current + 2);
  if (start > 2) out.push("…");
  for (let i = start; i <= end; i++) out.push(i);
  if (end < total - 1) out.push("…");
  out.push(total);
  return out;
}

function PageButton({
  onClick,
  disabled,
  active,
  ariaCurrent,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  ariaCurrent?: "page";
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-current={ariaCurrent}
      className={`min-w-7 rounded border px-2 py-1 text-xs transition-colors ${
        active
          ? "border-accent/40 bg-accent/10 text-accent"
          : "border-white/15 text-white/80 hover:bg-white/5"
      } disabled:cursor-not-allowed disabled:opacity-40`}
    >
      {children}
    </button>
  );
}
