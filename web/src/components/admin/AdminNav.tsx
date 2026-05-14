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
      { href: "/admin", label: "Dashboard" },
      { href: "/admin/status/alerts", label: "Alerts" },
    ],
    disabled: [],
  },
  {
    title: "Server",
    items: [
      { href: "/admin/server/general", label: "General" },
      { href: "/admin/server/network", label: "Network" },
      { href: "/admin/server/transcoder", label: "Transcoder" },
      { href: "/admin/server/webhooks", label: "Webhooks" },
      { href: "/admin/server/privacy", label: "Privacy" },
    ],
    disabled: [],
  },
  {
    title: "Library",
    items: [
      { href: "/admin/library/libraries", label: "Libraries" },
      { href: "/admin/library/agents", label: "Metadata Agents" },
      { href: "/admin/library/scheduled-tasks", label: "Scheduled Tasks" },
      { href: "/admin/library/optimized", label: "Optimized Versions" },
    ],
    disabled: [],
  },
  {
    title: "Users",
    items: [
      { href: "/admin/users/users", label: "Users" },
      { href: "/admin/users/invites", label: "Invites" },
      { href: "/admin/users/access", label: "Access" },
      { href: "/admin/users/devices", label: "Devices" },
    ],
    disabled: [],
  },
  {
    title: "Maintenance",
    items: [
      { href: "/admin/maintenance/audit", label: "Audit Log" },
      { href: "/admin/maintenance/backup", label: "Backup" },
      { href: "/admin/maintenance/logs", label: "Logs" },
    ],
    disabled: [],
  },
];

export function AdminNav() {
  const pathname = usePathname() ?? "";

  return (
    <nav className="flex flex-col gap-6 text-sm">
      {GROUPS.map((group) => (
        <div key={group.title}>
          <div className="mb-2 px-3 text-xs font-semibold uppercase tracking-wider text-white/40">
            {group.title}
          </div>
          <ul className="flex flex-col gap-px">
            {group.items.map((item) => {
              const isActive =
                item.href === pathname ||
                (item.href !== "/admin" && pathname.startsWith(item.href));
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
