import type { ReactNode } from "react";

/// Filter chip used above tables (Users, Audit log, Devices). Pair
/// with a search input on the same row. `count` renders as a small
/// pill-shaped badge after the label so the chip can't wrap. Each
/// chip is `nowrap`; the parent should `flex-wrap` so the bar
/// reflows but no individual chip splits.
export function FilterChip({
  active,
  count,
  onClick,
  children,
}: {
  active?: boolean;
  count?: number | string;
  onClick?: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={!!active}
      className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-[12px] transition-colors ${
        active
          ? "border-accent/30 bg-accent/10 text-accent"
          : "border-white/10 bg-white/4 text-white/70 hover:border-white/20 hover:text-white"
      }`}
    >
      <span>{children}</span>
      {count !== undefined && (
        <span
          className={`min-w-4.5 rounded-full px-1.5 text-center text-[11px] ${
            active ? "bg-accent/20 text-accent" : "bg-white/6 text-white/50"
          }`}
        >
          {count}
        </span>
      )}
    </button>
  );
}
