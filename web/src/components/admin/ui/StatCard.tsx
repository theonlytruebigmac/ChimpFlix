import type { ReactNode } from "react";

export type StatTone = "red" | "green" | "blue" | "amber" | "violet";

const TONE: Record<StatTone, string> = {
  red: "bg-accent/15 text-accent",
  green: "bg-emerald-500/[0.13] text-emerald-300",
  blue: "bg-blue-500/[0.13] text-blue-300",
  amber: "bg-amber-500/[0.13] text-amber-300",
  violet: "bg-violet-500/[0.13] text-violet-300",
};

/// Compact metric card for dashboards and the Activity page: a tinted icon,
/// a label, a big value, optional sub-meta, and an optional progress bar
/// (e.g. disk usage). `HeroCard` is the larger striped dashboard variant;
/// reach for StatCard inside grids of 3–5 numbers.
export function StatCard({
  label,
  value,
  meta,
  icon,
  tone = "blue",
  bar,
  className = "",
}: {
  label: ReactNode;
  value: ReactNode;
  meta?: ReactNode;
  icon?: ReactNode;
  tone?: StatTone;
  /// Optional progress bar, 0–100.
  bar?: number;
  className?: string;
}) {
  return (
    <div
      className={`rounded-lg border border-white/10 bg-gradient-to-b from-white/[0.04] to-transparent p-4 ${className}`}
    >
      <div className="flex items-center gap-2.5 text-[12.5px] font-medium text-white/60">
        {icon && (
          <span className={`grid h-7 w-7 shrink-0 place-items-center rounded-lg ${TONE[tone]}`}>
            {icon}
          </span>
        )}
        {label}
      </div>
      <div className="mt-3 text-2xl font-extrabold tracking-tight">{value}</div>
      {meta && <div className="mt-1 text-[12px] text-white/45">{meta}</div>}
      {bar != null && (
        <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-white/8">
          <div
            className="h-full rounded-full bg-(--color-accent)"
            style={{ width: `${Math.max(0, Math.min(100, bar))}%` }}
          />
        </div>
      )}
    </div>
  );
}
