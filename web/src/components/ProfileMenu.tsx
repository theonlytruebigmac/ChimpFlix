"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import {
  ChimpFlixApiError,
  auth as authApi,
  type User,
} from "@/lib/chimpflix-api";

// Session-scoped cache for the /api/v1/auth/me result. Avoids the second
// network hit on every navigation; identity rarely changes mid-session.
const AUTH_ME_KEY = "cf_auth_me_v2";
const AUTH_ME_TTL_MS = 5 * 60_000;

function readCachedUser(): User | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem(AUTH_ME_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { v: User; t: number };
    if (Date.now() - parsed.t > AUTH_ME_TTL_MS) return null;
    return parsed.v;
  } catch {
    return null;
  }
}

function writeCachedUser(value: User | null): void {
  if (typeof window === "undefined") return;
  try {
    if (value) {
      window.sessionStorage.setItem(
        AUTH_ME_KEY,
        JSON.stringify({ v: value, t: Date.now() }),
      );
    } else {
      window.sessionStorage.removeItem(AUTH_ME_KEY);
    }
  } catch {
    // sessionStorage can throw under privacy modes — fall back to fetch.
  }
}

export function ProfileMenu() {
  const router = useRouter();
  const [user, setUser] = useState<User | null>(null);
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    const cached = readCachedUser();
    // Hydrate from localStorage cache synchronously so the avatar
    // doesn't flash empty on every navigation. The async refresh
    // below overwrites with fresh server data when it lands.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (cached) setUser(cached);
    authApi
      .me()
      .then((res) => {
        if (cancelled) return;
        setUser(res.user);
        writeCachedUser(res.user);
      })
      .catch((e) => {
        if (cancelled) return;
        if (e instanceof ChimpFlixApiError && e.status === 401) {
          writeCachedUser(null);
          // Don't redirect from here — pages that need auth do their own
          // redirects server-side. The menu just hides.
          setUser(null);
        }
      });
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

  const initial = (user?.display_name ?? user?.username ?? "?")
    .charAt(0)
    .toUpperCase();
  const label = user?.display_name ?? user?.username ?? "";
  const isOwner = user?.role === "owner";

  async function onSignOut() {
    setOpen(false);
    try {
      await authApi.logout();
    } catch {
      // Best-effort — even if the server didn't ack, clear local state and
      // bounce to /login. The cookie has a short expiry anyway.
    }
    writeCachedUser(null);
    setUser(null);
    router.push("/login");
    router.refresh();
  }

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-label={user ? `Signed in as ${label}` : "Account"}
        aria-haspopup="menu"
        aria-expanded={open}
        className="flex items-center gap-1.5 rounded-md text-white/85 transition-colors hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-(--color-accent) focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        {user?.avatar_url ? (
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={user.avatar_url}
            alt=""
            width={28}
            height={28}
            className="h-7 w-7 rounded-md object-cover"
          />
        ) : (
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-(--color-accent)/80 text-xs font-semibold text-white">
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
              <div className="truncate text-sm font-medium">{label}</div>
              <div className="mt-0.5 truncate text-xs text-white/60">
                {isOwner ? "Owner" : "User"}
              </div>
            </div>
          )}
          <Link
            href="/history"
            role="menuitem"
            onClick={() => setOpen(false)}
            className="block px-4 py-2.5 text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white focus:outline-none focus-visible:bg-white/10 focus-visible:text-white"
          >
            Watch history
          </Link>
          <Link
            href="/settings"
            role="menuitem"
            onClick={() => setOpen(false)}
            className="block px-4 py-2.5 text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white focus:outline-none focus-visible:bg-white/10 focus-visible:text-white"
          >
            Settings
          </Link>
          <button
            type="button"
            role="menuitem"
            onClick={onSignOut}
            className="block w-full cursor-pointer px-4 py-2.5 text-left text-sm text-white/85 transition-colors hover:bg-white/10 hover:text-white focus:outline-none focus-visible:bg-white/10 focus-visible:text-white"
          >
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}
