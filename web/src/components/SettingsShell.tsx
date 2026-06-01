"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { ContextSwitcher } from "@/components/admin/ui/ContextSwitcher";
import { CommandPalette, type CommandItem } from "@/components/CommandPalette";

interface NavItem {
  href: string;
  label: string;
}
interface NavGroup {
  title: string;
  items: NavItem[];
}

const YOU_GROUPS: NavGroup[] = [
  {
    title: "Your account",
    items: [
      { href: "/settings/account", label: "Account" },
      { href: "/settings/playback", label: "Playback" },
      { href: "/settings/integrations", label: "Integrations" },
    ],
  },
  {
    title: "Preferences",
    items: [
      { href: "/settings/notifications", label: "Notifications" },
      { href: "/settings/home", label: "Home & visibility" },
      { href: "/settings/devices", label: "Devices & sessions" },
    ],
  },
];

const SERVER_GROUPS: NavGroup[] = [
  {
    title: "Operate",
    items: [
      { href: "/settings/admin/overview", label: "Overview" },
      { href: "/settings/admin/activity", label: "Activity & stats" },
      { href: "/settings/admin/status/alerts", label: "Alerts" },
    ],
  },
  {
    title: "Server",
    items: [
      { href: "/settings/admin/general", label: "General" },
      { href: "/settings/admin/network", label: "Network" },
      { href: "/settings/admin/transcoding", label: "Transcoding" },
      { href: "/settings/admin/credentials", label: "Credentials" },
      { href: "/settings/admin/notifications", label: "Notifications" },
    ],
  },
  {
    title: "Library",
    items: [
      { href: "/settings/admin/libraries", label: "Libraries" },
      { href: "/settings/admin/tasks", label: "Tasks & jobs" },
    ],
  },
  {
    title: "Users",
    items: [{ href: "/settings/admin/users", label: "Users" }],
  },
  {
    title: "Maintenance",
    items: [
      { href: "/settings/admin/maintenance", label: "Maintenance" },
      { href: "/settings/admin/logs", label: "Logs & audit" },
    ],
  },
];

/// Unified settings sidebar shell. One left nav that switches between the
/// personal ("You") and server ("Server", owner-only) contexts based on the
/// path. Owns the ⌘K command palette + global key listener, and a slide-in
/// drawer on small screens (the sticky sidebar is desktop-only).
export function SettingsShell({ isOwner }: { isOwner: boolean }) {
  const pathname = usePathname() ?? "";
  const context: "you" | "server" = pathname.startsWith("/settings/admin")
    ? "server"
    : "you";
  const groups = context === "server" ? SERVER_GROUPS : YOU_GROUPS;

  // Longest-match wins so a child route highlights only its own item.
  const activeHref = useMemo(() => {
    const all = groups.flatMap((g) => g.items.map((i) => i.href));
    return all
      .filter((h) => h === pathname || pathname.startsWith(`${h}/`))
      .reduce((longest, h) => (h.length > longest.length ? h : longest), "");
  }, [groups, pathname]);

  const [paletteOpen, setPaletteOpen] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);

  // Global ⌘K. Esc also closes the mobile drawer.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen(true);
      }
      if (e.key === "Escape") setDrawerOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const commandItems: CommandItem[] = useMemo(() => {
    const you = YOU_GROUPS.flatMap((g) =>
      g.items.map((i) => ({ ...i, group: "You" })),
    );
    const server = isOwner
      ? SERVER_GROUPS.flatMap((g) => g.items.map((i) => ({ ...i, group: "Server" })))
      : [];
    return [...you, ...server];
  }, [isOwner]);

  const searchButton = (
    <button
      type="button"
      onClick={() => setPaletteOpen(true)}
      className="flex w-full items-center gap-2 rounded-md border border-white/10 bg-white/4 px-3 py-2 text-[13px] text-white/45 transition-colors hover:border-white/20 hover:text-white/70"
    >
      <SearchIcon />
      <span>Search settings…</span>
      <kbd className="ml-auto rounded border border-white/10 bg-white/8 px-1.5 py-0.5 text-[10px] font-semibold text-white/60">
        ⌘K
      </kbd>
    </button>
  );

  const nav = (
    <nav className="flex flex-col gap-6 text-sm">
      {groups.map((group) => (
        <div key={group.title}>
          <div className="mb-2 px-3 text-[11px] font-semibold uppercase tracking-wider text-white/40">
            {group.title}
          </div>
          <ul className="flex flex-col gap-px">
            {group.items.map((item) => {
              const active = item.href === activeHref;
              return (
                <li key={item.href}>
                  <Link
                    href={item.href}
                    aria-current={active ? "page" : undefined}
                    onClick={() => setDrawerOpen(false)}
                    className={`block rounded-md px-3 py-1.5 transition-colors ${
                      active
                        ? "bg-white/10 text-white"
                        : "text-white/70 hover:bg-white/5 hover:text-white"
                    }`}
                  >
                    {item.label}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );

  const navColumn = (
    <>
      <ContextSwitcher
        context={context}
        canAccessServer={isOwner}
        serverHref="/settings/admin/overview"
      />
      <div className="mt-4">{searchButton}</div>
      <div className="mt-5">{nav}</div>
    </>
  );

  return (
    <>
      {/* Mobile bar: menu trigger + current context + quick search */}
      <div className="mb-2 flex items-center gap-2 lg:hidden">
        <button
          type="button"
          onClick={() => setDrawerOpen(true)}
          aria-label="Open settings menu"
          className="flex items-center gap-2 rounded-md border border-white/10 bg-white/4 px-3 py-2 text-[13px] font-medium text-white/80 transition-colors hover:bg-white/8"
        >
          <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <path d="M4 6h16M4 12h16M4 18h16" />
          </svg>
          {context === "server" ? "Server settings" : "Your settings"}
        </button>
        <button
          type="button"
          onClick={() => setPaletteOpen(true)}
          aria-label="Search settings"
          className="ml-auto flex h-9 w-9 items-center justify-center rounded-md border border-white/10 bg-white/4 text-white/60 transition-colors hover:text-white"
        >
          <SearchIcon />
        </button>
      </div>

      {/* Desktop sticky sidebar */}
      <div className="hidden lg:sticky lg:top-28 lg:block">{navColumn}</div>

      {/* Mobile slide-in drawer */}
      {drawerOpen && (
        <div
          className="fixed inset-0 z-55 bg-black/60 lg:hidden"
          onClick={() => setDrawerOpen(false)}
          role="dialog"
          aria-modal="true"
          aria-label="Settings menu"
        >
          <div
            className="zf-rise-in absolute inset-y-0 left-0 w-72 max-w-[85vw] overflow-y-auto border-r border-white/10 bg-(--color-surface) p-4"
            onClick={(e) => e.stopPropagation()}
          >
            {navColumn}
          </div>
        </div>
      )}

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        items={commandItems}
      />
    </>
  );
}

function SearchIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
    </svg>
  );
}
