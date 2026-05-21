import type { ReactNode } from "react";
import type { PillTone } from "./Pill";

/// Top-of-dashboard stat card. A narrow vertical accent stripe on the
/// left signals overall health (`tone`); the body has an eyebrow
/// label, a big value, and a meta line for context. Optional sparkline
/// slot below for a tiny inline trend chart.
///
/// Stretches to fill its grid cell, so put 3 (or N) of these in a
/// `grid-cols-3` and they line up automatically.
export function HeroCard({
  tone = "muted",
  label,
  icon,
  value,
  meta,
  spark,
  href,
}: {
  tone?: PillTone;
  label: string;
  /// Small SVG icon rendered next to the eyebrow label. Pass an
  /// already-coloured SVG; we don't override stroke.
  icon?: ReactNode;
  /// The big number. Strings rather than numbers so callers can format
  /// (e.g. "Healthy", "3 / 12 online").
  value: ReactNode;
  /// Single-line caption below the value (e.g. "CPU 14% · RAM 3.1 / 16 GB").
  meta?: ReactNode;
  /// Inline sparkline SVG (32px tall). Optional.
  spark?: ReactNode;
  /// Make the whole card a link to a detail page when set.
  href?: string;
}) {
  const stripe: Record<PillTone, string> = {
    ok:     "bg-emerald-500",
    warn:   "bg-amber-500",
    bad:    "bg-red-500",
    info:   "bg-blue-500",
    muted:  "bg-white/15",
    accent: "bg-accent",
  };
  const Wrapper = href ? "a" : "div";
  const wrapperProps = href ? { href } : {};
  return (
    <Wrapper
      {...wrapperProps}
      className={`relative block overflow-hidden rounded-lg border border-white/10 bg-white/2 p-4 ${
        href ? "transition-colors hover:bg-white/4" : ""
      }`}
    >
      <span aria-hidden className={`absolute inset-y-0 left-0 w-[3px] ${stripe[tone]}`} />
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.06em] text-white/55">
        {icon}
        {label}
      </div>
      <div className="text-[28px] font-bold leading-none tracking-tight">
        {value}
      </div>
      {meta && <div className="mt-1.5 text-[12px] text-white/55">{meta}</div>}
      {spark && <div className="mt-2 h-8 w-full">{spark}</div>}
    </Wrapper>
  );
}
