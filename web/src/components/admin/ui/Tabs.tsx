"use client";

import type { ReactNode } from "react";

export interface TabItem {
  id: string;
  label: ReactNode;
  /// Optional count badge (e.g. "Sessions 3").
  count?: number | string;
  /// Render visible-but-inert — used for owner-only tabs shown to admins.
  disabled?: boolean;
}

/// In-page tab bar (state-controlled). Unlike `AdminTabBar`, which links
/// between sub-routes, this switches panels within a single consolidated
/// page (the new Libraries / Tasks / Users / Maintenance pages). The parent
/// owns `active` so it can also sync to a `?tab=` query param for deep links.
export function Tabs({
  tabs,
  active,
  onSelect,
  className = "",
}: {
  tabs: TabItem[];
  active: string;
  onSelect: (id: string) => void;
  className?: string;
}) {
  return (
    <div className={`mb-6 border-b border-white/10 ${className}`}>
      <nav aria-label="Section" className="flex flex-wrap gap-1 overflow-x-auto">
        {tabs.map((tab) => {
          const on = tab.id === active;
          return (
            <button
              key={tab.id}
              type="button"
              disabled={tab.disabled}
              aria-current={on ? "page" : undefined}
              onClick={() => !tab.disabled && onSelect(tab.id)}
              className={`-mb-px flex items-center gap-2 whitespace-nowrap border-b-2 px-3 py-2 text-[13px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                on
                  ? "border-(--color-accent) text-white"
                  : "border-transparent text-white/60 hover:border-white/15 hover:text-white"
              }`}
            >
              {tab.label}
              {tab.count != null && (
                <span
                  className={`rounded-full px-1.5 text-[11px] font-semibold ${
                    on ? "bg-accent/15 text-accent" : "bg-white/8 text-white/60"
                  }`}
                >
                  {tab.count}
                </span>
              )}
            </button>
          );
        })}
      </nav>
    </div>
  );
}
