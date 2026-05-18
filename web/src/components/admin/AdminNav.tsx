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
      { href: "/settings/admin", label: "Dashboard" },
      { href: "/settings/admin/status/alerts", label: "Alerts" },
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
      { href: "/settings/admin/server/email", label: "Email" },
      { href: "/settings/admin/server/webhooks", label: "Webhooks" },
      { href: "/settings/admin/server/privacy", label: "Privacy" },
    ],
    disabled: [],
  },
  {
    title: "Library",
    items: [
      { href: "/settings/admin/library/libraries", label: "Libraries" },
      { href: "/settings/admin/library/agents", label: "Metadata Agents" },
      { href: "/settings/admin/library/scheduled-tasks", label: "Scheduled Tasks" },
      { href: "/settings/admin/library/optimized", label: "Optimized Versions" },
    ],
    disabled: [],
  },
  {
    title: "Users",
    items: [
      { href: "/settings/admin/users/users", label: "Users" },
      { href: "/settings/admin/users/invites", label: "Invites" },
      { href: "/settings/admin/users/access", label: "Access" },
      { href: "/settings/admin/users/groups", label: "Groups" },
      { href: "/settings/admin/users/devices", label: "Devices" },
    ],
    disabled: [],
  },
  {
    title: "Maintenance",
    items: [
      { href: "/settings/admin/maintenance", label: "Overview" },
      { href: "/settings/admin/maintenance/audit", label: "Audit Log" },
      { href: "/settings/admin/maintenance/backup", label: "Backup" },
      { href: "/settings/admin/maintenance/logs", label: "Logs" },
      { href: "/settings/admin/maintenance/health", label: "Library Health" },
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
                (item.href !== "/settings/admin" && pathname.startsWith(item.href));
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
