"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

/// Horizontal tab bar for consolidated admin pages (Users, Logs,
/// Notifications, Scheduled Tasks). Renders one underlined tab per
/// `tab` entry; the active tab is detected via `usePathname()` so
/// each sub-route gets it for free without prop-drilling.
///
/// Use under `AdminPageHeader` at the top of a layout.tsx; the
/// matching sub-page renders below.
export interface AdminTab {
  href: string;
  label: string;
  /// Override the default longest-prefix match (e.g. tab A is at
  /// "/foo" and tab B is at "/foo/bar" — without this both match
  /// when the path is "/foo/bar"). Defaults to exact-or-startsWith.
  match?: (pathname: string) => boolean;
}

export function AdminTabBar({ tabs }: { tabs: AdminTab[] }) {
  const pathname = usePathname() ?? "";
  // Longest-match wins, same algorithm as the sidebar. Prevents the
  // parent tab from co-highlighting when the user is on a child.
  const activeHref = tabs
    .map((t) => t.href)
    .filter((h) => h === pathname || pathname.startsWith(`${h}/`))
    .reduce((longest, h) => (h.length > longest.length ? h : longest), "");

  return (
    <div className="mb-6 -mt-2 border-b border-white/10">
      <nav
        aria-label="Section"
        className="flex flex-wrap gap-1 overflow-x-auto"
      >
        {tabs.map((tab) => {
          const active = tab.match
            ? tab.match(pathname)
            : tab.href === activeHref;
          return (
            <Link
              key={tab.href}
              href={tab.href}
              aria-current={active ? "page" : undefined}
              className={`-mb-px whitespace-nowrap border-b-2 px-3 py-2 text-[13px] font-medium transition-colors ${
                active
                  ? "border-(--color-accent) text-white"
                  : "border-transparent text-white/60 hover:border-white/15 hover:text-white"
              }`}
            >
              {tab.label}
            </Link>
          );
        })}
      </nav>
    </div>
  );
}
