"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useMemo, useRef, useState } from "react";
import { ContextSwitcher } from "@/components/admin/ui/ContextSwitcher";
import { CommandPalette, type CommandItem } from "@/components/CommandPalette";

interface NavItem {
  href: string;
  label: string;
  icon: keyof typeof NAV_ICONS;
}
interface NavGroup {
  title: string;
  items: NavItem[];
}

const YOU_GROUPS: NavGroup[] = [
  {
    title: "Your account",
    items: [
      { href: "/settings/account", label: "Account", icon: "account" },
      { href: "/settings/playback", label: "Playback", icon: "playback" },
      { href: "/settings/integrations", label: "Integrations", icon: "integrations" },
    ],
  },
  {
    title: "Preferences",
    items: [
      { href: "/settings/notifications", label: "Notifications", icon: "bell" },
      { href: "/settings/home", label: "Home & visibility", icon: "home" },
      { href: "/settings/devices", label: "Devices & sessions", icon: "devices" },
    ],
  },
];

const SERVER_GROUPS: NavGroup[] = [
  {
    title: "Operate",
    items: [
      { href: "/settings/admin/overview", label: "Overview", icon: "overview" },
      { href: "/settings/admin/activity", label: "Activity & stats", icon: "activity" },
      { href: "/settings/admin/status/alerts", label: "Alerts", icon: "alerts" },
    ],
  },
  {
    title: "Server",
    items: [
      { href: "/settings/admin/general", label: "General", icon: "general" },
      { href: "/settings/admin/network", label: "Network", icon: "network" },
      { href: "/settings/admin/transcoding", label: "Transcoding", icon: "transcoding" },
      { href: "/settings/admin/credentials", label: "Credentials", icon: "credentials" },
      { href: "/settings/admin/notifications", label: "Notifications", icon: "mail" },
    ],
  },
  {
    title: "Library",
    items: [
      { href: "/settings/admin/libraries", label: "Libraries", icon: "libraries" },
      { href: "/settings/admin/tasks", label: "Tasks & jobs", icon: "tasks" },
    ],
  },
  {
    title: "Users",
    items: [{ href: "/settings/admin/users", label: "Users", icon: "users" }],
  },
  {
    title: "Maintenance",
    items: [
      { href: "/settings/admin/maintenance", label: "Maintenance", icon: "maintenance" },
      { href: "/settings/admin/logs", label: "Logs & audit", icon: "logs" },
    ],
  },
];

/// Stroke icons for the sidebar nav, keyed by NavItem.icon. 24px viewBox,
/// rendered at 17px by `.cf-nav-item svg`. Kept here (not a shared icon
/// set) because these are settings-nav-specific.
const NAV_ICONS = {
  account: <><circle cx="12" cy="8" r="4" /><path d="M4 20a8 8 0 0 1 16 0" /></>,
  playback: <path d="M7 5v14l11-7z" />,
  integrations: <><path d="M9 7L5 11a4 4 0 0 0 6 6l1-1" /><path d="M15 17l4-4a4 4 0 0 0-6-6l-1 1" /></>,
  bell: <><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" /><path d="M10.5 21a1.5 1.5 0 0 0 3 0" /></>,
  home: <><path d="M3 11l9-8 9 8" /><path d="M5 10v10h14V10" /></>,
  devices: <><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8M12 16v4" /></>,
  overview: <><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></>,
  activity: <path d="M3 12h4l3 8 4-16 3 8h4" />,
  alerts: <><path d="M12 3l9 16H3z" /><path d="M12 10v4M12 17v.5" /></>,
  general: <><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2" /></>,
  network: <><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18" /></>,
  transcoding: <><rect x="6" y="6" width="12" height="12" rx="2" /><path d="M9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2" /></>,
  credentials: <><circle cx="8" cy="15" r="4" /><path d="M11 12l8-8 2 2-2 2 2 2-3 3-2-2-3 3" /></>,
  mail: <><rect x="3" y="5" width="18" height="14" rx="2" /><path d="M3 7l9 6 9-6" /></>,
  libraries: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  tasks: <><path d="M9 6h11M9 12h11M9 18h11" /><path d="M4 6l1 1 2-2M4 12l1 1 2-2M4 18l1 1 2-2" /></>,
  users: <><circle cx="9" cy="8" r="3" /><path d="M3 20a6 6 0 0 1 12 0" /><path d="M16 5.5a3 3 0 0 1 0 6M21 20a6 6 0 0 0-4-5.6" /></>,
  maintenance: <path d="M14 7a4 4 0 0 0-5.5 5.5L4 17l3 3 4.5-4.5A4 4 0 0 0 17 10l-2.5 2.5-2.5-2.5z" />,
  logs: <><path d="M6 2h9l5 5v15H6z" /><path d="M14 2v6h6M9 13h7M9 17h7" /></>,
} as const;

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

  // Refs for focus management: the panel holds focusable children; the trigger
  // button receives focus back when the drawer closes.
  const drawerPanelRef = useRef<HTMLDivElement>(null);
  const menuTriggerRef = useRef<HTMLButtonElement>(null);

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

  // Focus trap: when the drawer opens, move focus to the first focusable element
  // inside the panel; when it closes, return focus to the trigger button.
  useEffect(() => {
    if (!drawerOpen) {
      menuTriggerRef.current?.focus();
      return;
    }

    const panel = drawerPanelRef.current;
    if (!panel) return;

    const focusableSelectors =
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

    // Move focus into the drawer on open.
    const firstFocusable = panel.querySelector<HTMLElement>(focusableSelectors);
    firstFocusable?.focus();

    // Trap Tab / Shift+Tab within the panel while the drawer is open.
    const trapFocus = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const focusable = Array.from(panel.querySelectorAll<HTMLElement>(focusableSelectors));
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey) {
        if (document.activeElement === first) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };

    panel.addEventListener("keydown", trapFocus);
    return () => panel.removeEventListener("keydown", trapFocus);
  }, [drawerOpen]);

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
    <nav className="flex flex-col gap-2">
      {groups.map((group) => (
        <div key={group.title} className="cf-nav-group">
          <div className="cf-nav-label">{group.title}</div>
          <ul>
            {group.items.map((item) => {
              const active = item.href === activeHref;
              return (
                <li key={item.href}>
                  <Link
                    href={item.href}
                    aria-current={active ? "page" : undefined}
                    onClick={() => setDrawerOpen(false)}
                    className={`cf-nav-item${active ? " cf-active" : ""}`}
                  >
                    <svg
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      aria-hidden
                    >
                      {NAV_ICONS[item.icon]}
                    </svg>
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
          ref={menuTriggerRef}
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
            ref={drawerPanelRef}
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
