"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import {
  ChimpFlixApiError,
  libraries as librariesApi,
  prefs as prefsApi,
} from "@/lib/chimpflix-api";

interface NavLibrary {
  id: number;
  name: string;
}

const INLINE_LIBRARY_LIMIT = 4;

// Session-scoped cache for the nav library list. Mirrors ProfileMenu —
// avoids re-fetching on every navigation since the visible-libraries set
// changes rarely (admin actions, prefs toggles).
const NAV_LIBS_KEY = "cf_nav_libs_v1";
const NAV_LIBS_TTL_MS = 60_000;

function readCached(): NavLibrary[] | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(NAV_LIBS_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { v: NavLibrary[]; t: number };
    if (Date.now() - parsed.t > NAV_LIBS_TTL_MS) return null;
    return parsed.v;
  } catch {
    return null;
  }
}

function writeCached(value: NavLibrary[] | null): void {
  if (typeof window === "undefined") return;
  try {
    if (value) {
      window.sessionStorage.setItem(
        NAV_LIBS_KEY,
        JSON.stringify({ v: value, t: Date.now() }),
      );
    } else {
      window.sessionStorage.removeItem(NAV_LIBS_KEY);
    }
  } catch {
    // sessionStorage can throw under privacy modes — fall back to fetch.
  }
}

interface NavItem {
  href: string;
  label: string;
  match: (path: string) => boolean;
}

function libraryItem(lib: NavLibrary): NavItem {
  const href = `/library/${lib.id}`;
  return {
    href,
    label: lib.name,
    match: (p) => p === href || p.startsWith(`${href}/`),
  };
}

/// Shared hook — fetches the user's visible libraries once and caches
/// in sessionStorage. Both the desktop bar and mobile drawer consume it.
function useNavLibraries(): NavLibrary[] {
  // Empty array initial so SSR + first client render match. The cached
  // value from sessionStorage gets applied in the effect below — same
  // pattern as the role hook above, same reason (hydration mismatch
  // crashed mobile Chrome's renderer).
  const [libraries, setLibraries] = useState<NavLibrary[]>([]);

  useEffect(() => {
    let cancelled = false;
    const cached = readCached();
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (cached) setLibraries(cached);
    Promise.all([librariesApi.list(), prefsApi.hiddenLibraries()])
      .then(([{ libraries: all }, { library_ids: hiddenIds }]) => {
        if (cancelled) return;
        const hidden = new Set(hiddenIds);
        const visible = all
          .filter(
            (l) => l.visibility === "home_and_search" && !hidden.has(l.id),
          )
          .map((l) => ({ id: l.id, name: l.name }));
        setLibraries(visible);
        writeCached(visible);
      })
      .catch((e) => {
        if (cancelled) return;
        if (e instanceof ChimpFlixApiError && (e.status === 401 || e.status === 403)) {
          writeCached(null);
          setLibraries([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return libraries;
}

function navItems(
  libraries: NavLibrary[],
  includeOverflow: boolean,
): {
  primary: NavItem[];
  overflow: NavItem[];
} {
  const inline = libraries.slice(0, INLINE_LIBRARY_LIMIT).map(libraryItem);
  const overflow = libraries.slice(INLINE_LIBRARY_LIMIT).map(libraryItem);

  const leading: NavItem[] = [
    { href: "/", label: "Home", match: (p) => p === "/" },
  ];
  const trailing: NavItem[] = [
    {
      href: "/new-popular",
      label: "New & Popular",
      match: (p) => p === "/new-popular" || p.startsWith("/new-popular/"),
    },
    {
      href: "/my-list",
      label: "My List",
      match: (p) => p === "/my-list" || p.startsWith("/my-list/"),
    },
  ];

  if (includeOverflow) {
    // Mobile drawer renders everything inline — no overflow split.
    return {
      primary: [...leading, ...inline, ...overflow, ...trailing],
      overflow: [],
    };
  }
  return { primary: [...leading, ...inline, ...trailing], overflow };
}

export function NavLinks() {
  const pathname = usePathname() ?? "";
  const libraries = useNavLibraries();
  const { primary, overflow } = navItems(libraries, false);

  return (
    <nav className="hidden items-center gap-5 text-sm md:flex">
      {primary.map((link) => (
        <NavLink key={link.href} item={link} active={link.match(pathname)} />
      ))}
      {overflow.length > 0 ? (
        <MoreMenu items={overflow} pathname={pathname} />
      ) : null}
    </nav>
  );
}

/// Mobile menu: hamburger button + slide-out drawer covering ~80% of
/// the viewport. Used in place of `NavLinks` when the desktop bar is
/// hidden (<md). Render alongside NavLinks in TopNav — the breakpoint
/// classes handle visibility.
export function MobileNavTrigger() {
  const pathname = usePathname() ?? "";
  const libraries = useNavLibraries();
  const { primary } = navItems(libraries, true);
  const [open, setOpen] = useState(false);

  // Close on navigation. usePathname changes when the user taps a
  // link inside the drawer; that's when we want it to dismiss.
  // The setState-in-effect is the explicit "external input changed
  // → reset local state" case the lint rule's documented exceptions
  // mention (responding to a prop/derived value change).
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setOpen(false);
  }, [pathname]);

  // Body-scroll lock while the drawer is open so the underlying page
  // doesn't scroll behind it.
  useEffect(() => {
    if (typeof document === "undefined") return;
    if (open) {
      const prev = document.body.style.overflow;
      document.body.style.overflow = "hidden";
      return () => {
        document.body.style.overflow = prev;
      };
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label="Open navigation menu"
        aria-expanded={open}
        className="flex h-10 w-10 items-center justify-center rounded-md text-white/85 transition-colors hover:bg-white/10 md:hidden"
      >
        <HamburgerIcon />
      </button>

      {open && (
        <div className="fixed inset-0 z-60 md:hidden">
          {/* Backdrop */}
          <button
            type="button"
            aria-label="Close menu"
            onClick={() => setOpen(false)}
            className="absolute inset-0 bg-black/70"
          />

          {/* Drawer */}
          <aside
            role="dialog"
            aria-label="Navigation"
            aria-modal="true"
            className="relative ml-0 flex h-full w-[80vw] max-w-xs flex-col bg-black shadow-2xl"
          >
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
              <span className="text-xs font-semibold uppercase tracking-wider text-white/50">
                Browse
              </span>
              <button
                type="button"
                onClick={() => setOpen(false)}
                aria-label="Close menu"
                className="flex h-9 w-9 items-center justify-center rounded-md text-white/70 hover:bg-white/10 hover:text-white"
              >
                <CloseIcon />
              </button>
            </div>

            <ul className="flex-1 overflow-y-auto py-3">
              {primary.map((item) => {
                const active = item.match(pathname);
                return (
                  <li key={item.href}>
                    <Link
                      href={item.href}
                      onClick={() => setOpen(false)}
                      aria-current={active ? "page" : undefined}
                      className={
                        "block px-5 py-3 text-base transition-colors " +
                        (active
                          ? "bg-white/10 font-semibold text-white"
                          : "text-white/85 hover:bg-white/5 hover:text-white")
                      }
                    >
                      {item.label}
                    </Link>
                  </li>
                );
              })}
            </ul>
          </aside>
        </div>
      )}
    </>
  );
}

function NavLink({ item, active }: { item: NavItem; active: boolean }) {
  return (
    <Link
      href={item.href}
      aria-current={active ? "page" : undefined}
      className={`rounded focus:outline-none focus-visible:ring-2 focus-visible:ring-(--color-accent) focus-visible:ring-offset-2 focus-visible:ring-offset-background ${
        active
          ? "font-medium text-white"
          : "text-white/70 transition-colors hover:text-white"
      }`}
    >
      {item.label}
    </Link>
  );
}

function MoreMenu({
  items,
  pathname,
}: {
  items: NavItem[];
  pathname: string;
}) {
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLDivElement | null>(null);
  const anyActive = items.some((i) => i.match(pathname));

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapperRef.current?.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={wrapperRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        className={
          "flex items-center gap-1 transition-colors " +
          (anyActive
            ? "font-medium text-white"
            : "text-white/70 hover:text-white")
        }
      >
        More
        <svg
          aria-hidden
          width="10"
          height="10"
          viewBox="0 0 10 10"
          className={`transition-transform ${open ? "rotate-180" : ""}`}
        >
          <path
            d="M1 3l4 4 4-4"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
      {open ? (
        <div
          role="menu"
          className="absolute left-1/2 top-full z-50 mt-2 -translate-x-1/2 min-w-44 rounded-md border border-white/10 bg-black/95 py-2 shadow-2xl backdrop-blur-sm"
        >
          {items.map((item) => {
            const active = item.match(pathname);
            return (
              <Link
                key={item.href}
                href={item.href}
                role="menuitem"
                aria-current={active ? "page" : undefined}
                onClick={() => setOpen(false)}
                className={
                  "block px-4 py-2 text-sm transition-colors " +
                  (active
                    ? "font-medium text-white"
                    : "text-white/80 hover:text-white")
                }
              >
                {item.label}
              </Link>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

function HamburgerIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden
    >
      <line x1="4" y1="7" x2="20" y2="7" />
      <line x1="4" y1="12" x2="20" y2="12" />
      <line x1="4" y1="17" x2="20" y2="17" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden
    >
      <line x1="6" y1="6" x2="18" y2="18" />
      <line x1="18" y1="6" x2="6" y2="18" />
    </svg>
  );
}
