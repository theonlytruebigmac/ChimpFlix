"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";

type User = {
  id: number;
  uuid: string;
  username: string;
  email: string;
  thumb: string;
};

type AuthMe = { user: User | null; isAdmin: boolean; hasAdmin: boolean };

// Session-scoped cache for /api/auth/me. The endpoint is ~30-100ms over the
// LAN but TopNav fires it on every page mount. Stashing the response in
// sessionStorage means the second navigation onward shows the avatar
// instantly. The TTL is short — this is identity, not preferences — but
// long enough to cover a normal browsing session without re-querying Plex.
const AUTH_ME_KEY = "cf_auth_me";
const AUTH_ME_TTL_MS = 5 * 60_000;

function readCachedAuthMe(): AuthMe | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(AUTH_ME_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { v: AuthMe; t: number };
    if (Date.now() - parsed.t > AUTH_ME_TTL_MS) return null;
    return parsed.v;
  } catch {
    return null;
  }
}

function writeCachedAuthMe(value: AuthMe): void {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(
      AUTH_ME_KEY,
      JSON.stringify({ v: value, t: Date.now() }),
    );
  } catch {
    // sessionStorage can throw under privacy modes / quota — fall back to
    // the live fetch on every nav, no big deal.
  }
}

export function ProfileMenu() {
  const [user, setUser] = useState<User | null>(null);
  const [isAdmin, setIsAdmin] = useState(false);
  const [hasAdmin, setHasAdmin] = useState(false);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    // Synchronously paint the cached identity so the avatar doesn't flash a
    // placeholder on every nav.
    const cached = readCachedAuthMe();
    if (cached) {
      if (cached.user) setUser(cached.user);
      setIsAdmin(cached.isAdmin);
      setHasAdmin(cached.hasAdmin);
    }
    fetch("/api/auth/me")
      .then((r) => (r.ok ? r.json() : null))
      .then((data: AuthMe | null) => {
        if (cancelled || !data) return;
        if (data.user) setUser(data.user);
        setIsAdmin(Boolean(data.isAdmin));
        setHasAdmin(Boolean(data.hasAdmin));
        writeCachedAuthMe(data);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!containerRef.current?.contains(e.target as Node)) setOpen(false);
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open]);

  // Render a placeholder avatar before /api/auth/me resolves so the layout
  // stays stable instead of popping in.
  const initial = (user?.username ?? "?").charAt(0).toUpperCase();
  const avatarSrc = user?.thumb;

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label={user ? `Signed in as ${user.username}` : "Account"}
        aria-haspopup="menu"
        aria-expanded={open}
        className="flex items-center gap-1.5 text-white/85 transition-colors hover:text-white"
      >
        {avatarSrc ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={avatarSrc}
            alt=""
            className="h-7 w-7 rounded-md object-cover"
          />
        ) : (
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-white/15 text-xs font-semibold">
            {initial}
          </div>
        )}
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          aria-hidden
          className={`transition-transform duration-150 ${
            open ? "rotate-180" : ""
          }`}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {open && (
        <div
          role="menu"
          className="absolute right-0 top-full mt-2 w-56 overflow-hidden rounded-md border border-white/10 bg-(--color-surface) shadow-2xl"
        >
          {user && (
            <div className="border-b border-white/10 px-4 py-3">
              <div className="truncate text-sm font-medium">
                {user.username}
              </div>
              <div className="mt-0.5 truncate text-xs text-white/60">
                {user.email}
              </div>
            </div>
          )}
          {/*
            Show "Switch profile" whenever an admin token exists alongside
            the session. For the admin themselves it leads to the full
            profile picker; for a managed user it's their path back to admin
            (the switch endpoint blocks lateral managed→managed jumps).
          */}
          {(isAdmin || hasAdmin) && (
            <Link
              href="/switch"
              role="menuitem"
              onClick={() => setOpen(false)}
              className="block px-4 py-2.5 text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white"
            >
              Switch profile
            </Link>
          )}
          <button
            type="button"
            role="menuitem"
            onClick={async () => {
              setOpen(false);
              try {
                await fetch("/api/auth/clear-server", { method: "POST" });
              } catch {
                // ignore — page nav below still does the right thing as
                // long as the cookie clears or the user re-signs in.
              }
              // ?manual=1 forces the picker UI even on single-server
              // accounts — without it, autoSelect would just re-pick
              // the connection the user is trying to escape from.
              window.location.href = "/select-server?manual=1";
            }}
            className="block w-full cursor-pointer px-4 py-2.5 text-left text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white"
          >
            Switch server
          </button>
          <Link
            href="/settings"
            role="menuitem"
            onClick={() => setOpen(false)}
            className="block px-4 py-2.5 text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white"
          >
            Settings
          </Link>
          <a
            href="https://app.plex.tv/desktop/#!/settings/account"
            target="_blank"
            rel="noreferrer"
            role="menuitem"
            className="block px-4 py-2.5 text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white"
          >
            Manage Plex account ↗
          </a>
          <form action="/api/auth/logout" method="post">
            <button
              type="submit"
              role="menuitem"
              className="block w-full cursor-pointer px-4 py-2.5 text-left text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white"
            >
              Sign out
            </button>
          </form>
        </div>
      )}
    </div>
  );
}
