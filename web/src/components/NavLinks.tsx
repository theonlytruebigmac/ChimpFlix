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

export function NavLinks() {
  const pathname = usePathname() ?? "";
  const [libraries, setLibraries] = useState<NavLibrary[]>(
    () => readCached() ?? [],
  );

  useEffect(() => {
    let cancelled = false;
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

  const items = [...leading, ...inline, ...trailing];

  return (
    <nav className="hidden items-center gap-5 text-sm md:flex">
      {items.map((link) => (
        <NavLink key={link.href} item={link} active={link.match(pathname)} />
      ))}
      {overflow.length > 0 ? (
        <MoreMenu items={overflow} pathname={pathname} />
      ) : null}
    </nav>
  );
}

function NavLink({ item, active }: { item: NavItem; active: boolean }) {
  return (
    <Link
      href={item.href}
      aria-current={active ? "page" : undefined}
      className={
        active
          ? "font-medium text-white"
          : "text-white/70 transition-colors hover:text-white"
      }
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
