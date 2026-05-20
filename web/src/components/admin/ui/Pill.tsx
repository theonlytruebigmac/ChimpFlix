import type { ReactNode } from "react";

export type PillTone =
  | "ok"
  | "warn"
  | "bad"
  | "info"
  | "muted"
  | "accent";

const TONE_CLASSES: Record<PillTone, string> = {
  ok:     "bg-emerald-500/[0.13] text-emerald-300",
  warn:   "bg-amber-500/[0.13]   text-amber-300",
  bad:    "bg-red-500/[0.14]     text-red-300",
  info:   "bg-blue-500/[0.13]    text-blue-300",
  muted:  "bg-white/8            text-white/70",
  accent: "bg-accent/15 text-accent",
};

/// Status badge used across the admin surface. Pair with `dot` for
/// at-a-glance status (active / idle / offline) where the colored
/// circle does the heavy lifting; pair without `dot` for category
/// labels like role tier or "via Family". `nowrap` is on by default
/// because pills inside table cells should never wrap their content.
export function Pill({
  tone = "muted",
  dot,
  children,
  className = "",
}: {
  tone?: PillTone;
  dot?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-full px-2 py-0.5 text-[11.5px] font-medium leading-[18px] ${TONE_CLASSES[tone]} ${className}`}
    >
      {dot && (
        <span
          aria-hidden
          className="inline-block h-1.5 w-1.5 rounded-full bg-current"
        />
      )}
      {children}
    </span>
  );
}

/// Bare status dot (no surrounding pill) — used in tables next to a
/// label, or in small spaces where a full pill is too noisy.
export function StatusDot({
  tone = "muted",
  pulse,
  className = "",
}: {
  tone?: PillTone;
  pulse?: boolean;
  className?: string;
}) {
  const color: Record<PillTone, string> = {
    ok:     "bg-emerald-400",
    warn:   "bg-amber-400",
    bad:    "bg-red-400",
    info:   "bg-blue-400",
    muted:  "bg-white/30",
    accent: "bg-accent",
  };
  return (
    <span
      aria-hidden
      className={`inline-block h-2 w-2 shrink-0 rounded-full ${color[tone]} ${pulse ? "animate-pulse" : ""} ${className}`}
    />
  );
}
