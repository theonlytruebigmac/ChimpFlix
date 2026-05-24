"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

interface NavItem {
  href: string;
  label: string;
}

interface NavGroup {
  title: string;
  items: NavItem[];
}

// The shape of the admin surface, sequenced to match the phased plan. Items
// that point at unbuilt pages render disabled (no <Link>) so the IA is
// visible from day one — Phase N work just enables the matching entry.
const GROUPS: Array<NavGroup & { disabled?: string[] }> = [
  {
    title: "Status",
    items: [
      // Home replaces the old Dashboard: hero stats, recent
      // activity, alerts, and quick actions all in one surface.
      // The standalone Alerts + Stats pages remain as deep-dive
      // views — Home links into them via the "View all"
      // affordances on each card.
      { href: "/settings/admin", label: "Home" },
      { href: "/settings/admin/status/alerts", label: "Alerts" },
      { href: "/settings/admin/status/stats", label: "Stats" },
    ],
    disabled: [],
  },
  {
    title: "Server",
    items: [
      { href: "/settings/admin/server/general", label: "General" },
      { href: "/settings/admin/server/network", label: "Network" },
      { href: "/settings/admin/server/transcoder", label: "Transcoder" },
      { href: "/settings/admin/server/credentials", label: "Credentials" },
      { href: "/settings/admin/server/preroll", label: "Pre-roll" },
      {
        href: "/settings/admin/server/notifications",
        label: "Notifications",
      },
    ],
    disabled: [],
  },
  {
    title: "Library",
    items: [
      { href: "/settings/admin/library", label: "Library Settings" },
      { href: "/settings/admin/library/libraries", label: "Libraries" },
      { href: "/settings/admin/library/collections", label: "Collections" },
      { href: "/settings/admin/library/agents", label: "Metadata Agents" },
      {
        href: "/settings/admin/library/scheduled-tasks",
        label: "Scheduled Tasks",
      },
      {
        href: "/settings/admin/library/versions",
        label: "Optimized Versions",
      },
    ],
    disabled: [],
  },
  {
    title: "Users",
    items: [{ href: "/settings/admin/users", label: "Users" }],
    disabled: [],
  },
  {
    title: "Maintenance",
    items: [
      { href: "/settings/admin/maintenance", label: "Overview" },
      { href: "/settings/admin/maintenance/backup", label: "Backup" },
      { href: "/settings/admin/maintenance/bulk", label: "Bulk operations" },
      { href: "/settings/admin/maintenance/logs", label: "Logs" },
    ],
    disabled: [],
  },
];

// Flat list of every nav href, computed once. Used to pick the single
// "deepest" matching item so a parent page (e.g. /settings/admin/library —
// "Library Settings") doesn't co-highlight when the user is on a child
// route (/settings/admin/library/agents). Naive `startsWith(item.href)`
// matched both.
const ALL_HREFS: string[] = GROUPS.flatMap((g) => g.items.map((i) => i.href));

export function AdminNav() {
  const pathname = usePathname() ?? "";
  // Longest-match wins. A child route matches both its own href (exact)
  // and every ancestor href (`startsWith(`${h}/`)`); keeping only the
  // longest one yields the most specific item.
  const activeHref = ALL_HREFS.filter(
    (h) => h === pathname || pathname.startsWith(`${h}/`),
  ).reduce((longest, h) => (h.length > longest.length ? h : longest), "");

  return (
    <nav className="flex flex-col gap-6 text-sm">
      {GROUPS.map((group) => (
        <div key={group.title}>
          <div className="mb-2 px-3 text-xs font-semibold uppercase tracking-wider text-white/40">
            {group.title}
          </div>
          <ul className="flex flex-col gap-px">
            {group.items.map((item) => {
              const isActive = item.href === activeHref;
              const isDisabled = group.disabled?.includes(item.href) ?? false;
              const base =
                "block rounded-md px-3 py-1.5 transition-colors";
              const cls = isDisabled
                ? `${base} cursor-not-allowed text-white/25`
                : isActive
                  ? `${base} bg-white/10 text-white`
                  : `${base} text-white/70 hover:bg-white/5 hover:text-white`;
              return (
                <li key={item.href}>
                  {isDisabled ? (
                    <span className={cls} title="Coming in a later phase">
                      {item.label}
                    </span>
                  ) : (
                    <Link href={item.href} className={cls}>
                      {item.label}
                    </Link>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}
