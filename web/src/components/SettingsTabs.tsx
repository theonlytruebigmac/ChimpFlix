"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

interface Tab {
  href: string;
  label: string;
  /// True when the active path "lives under" the tab. Used so a
  /// sub-route like /settings/admin/library/agents still highlights
  /// the Admin tab.
  match: (path: string) => boolean;
}

const USER_TABS: Tab[] = [
  {
    href: "/settings/account",
    label: "Account",
    match: (p) => p === "/settings" || p.startsWith("/settings/account"),
  },
  {
    href: "/settings/player",
    label: "Player",
    match: (p) => p.startsWith("/settings/player"),
  },
  {
    href: "/settings/integrations",
    label: "Integrations",
    match: (p) => p.startsWith("/settings/integrations"),
  },
  {
    href: "/settings/libraries",
    label: "Libraries",
    match: (p) => p.startsWith("/settings/libraries"),
  },
];

const ADMIN_TAB: Tab = {
  href: "/settings/admin",
  label: "Admin Console",
  match: (p) => p.startsWith("/settings/admin"),
};

export function SettingsTabs({ isOwner }: { isOwner: boolean }) {
  const pathname = usePathname() ?? "";
  const tabs = isOwner ? [...USER_TABS, ADMIN_TAB] : USER_TABS;

  return (
    <nav
      role="tablist"
      aria-label="Settings sections"
      className="flex gap-6 border-b border-white/10 overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden"
    >
      {tabs.map((tab) => {
        const active = tab.match(pathname);
        const isAdmin = tab.href === ADMIN_TAB.href;
        return (
          <Link
            key={tab.href}
            href={tab.href}
            role="tab"
            aria-selected={active}
            className={`group relative shrink-0 whitespace-nowrap px-1 pb-3 pt-1 text-sm transition-colors ${
              active ? "text-white" : "text-white/55 hover:text-white"
            }`}
          >
            <span className="flex items-center gap-1.5">
              {tab.label}
              {isAdmin && (
                <span className="rounded bg-red-500/20 px-1 py-px text-[9px] uppercase tracking-wider text-red-300">
                  Owner
                </span>
              )}
            </span>
            <span
              className={`absolute inset-x-0 -bottom-px h-0.5 rounded-full bg-(--color-accent) transition-opacity ${
                active ? "opacity-100" : "opacity-0"
              }`}
              aria-hidden
            />
          </Link>
        );
      })}
    </nav>
  );
}
