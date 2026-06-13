"use client";

import type { ReactNode } from "react";

export interface SegOption<T extends string> {
  value: T;
  label: ReactNode;
  /// Optional trailing count, e.g. a filter that shows how many rows match.
  count?: number | string;
}

/// Segmented control — a compact, exclusive button group. Use for in-content
/// choices that aren't navigation: status filters ("All / Queued / Failed"),
/// time windows ("7d / 30d / 90d"), small enums. For switching between page
/// sections use `Tabs`; for routing between sub-pages use `AdminTabBar`.
export function Seg<T extends string>({
  options,
  value,
  onChange,
  size = "md",
  accent = false,
  className = "",
  "aria-label": ariaLabel,
}: {
  options: SegOption<T>[];
  value: T;
  onChange: (value: T) => void;
  size?: "sm" | "md";
  /// Solid red highlight on the active segment instead of the neutral
  /// white wash. Use sparingly — e.g. a primary time-window selector.
  accent?: boolean;
  className?: string;
  /// Accessible name for the button group. Pass the same text as the
  /// visible label to the left so screen readers announce context.
  "aria-label"?: string;
}) {
  const pad = size === "sm" ? "px-2.5 py-1 text-[12px]" : "px-3 py-1.5 text-[13px]";
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className={`inline-flex gap-1 rounded-md border border-white/10 bg-white/4 p-1 ${className}`}
    >
      {options.map((opt) => {
        const on = opt.value === value;
        const onClass = accent ? "bg-(--color-accent) text-white" : "bg-white/12 text-white";
        return (
          <button
            key={opt.value}
            type="button"
            aria-pressed={on}
            onClick={() => onChange(opt.value)}
            className={`whitespace-nowrap rounded font-medium transition-colors ${pad} ${
              on ? onClass : "text-white/65 hover:text-white"
            }`}
          >
            {opt.label}
            {opt.count != null && (
              <span className="ml-1.5 text-[11px] opacity-70">{opt.count}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
