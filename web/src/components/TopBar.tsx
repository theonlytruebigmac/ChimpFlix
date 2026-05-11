"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { chimpflix } from "@/lib/api";
import { Brand } from "./Brand";

export function TopBar() {
  const [label, setLabel] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    chimpflix.auth
      .me()
      .then((r) => {
        if (!cancelled) setLabel(r.user.display_name ?? r.user.username);
      })
      .catch(() => {
        // Unauthenticated — the page will redirect to /login.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function logout() {
    try {
      await chimpflix.auth.logout();
    } catch {
      // ignore — we navigate away regardless
    }
    window.location.href = "/login";
  }

  return (
    <header className="fixed inset-x-0 top-0 z-50 bg-linear-to-b from-black/85 via-black/55 to-transparent">
      <div className="flex items-center gap-10 px-6 py-4 sm:px-12">
        <Link href="/" aria-label="Home">
          <Brand />
        </Link>
        <nav className="hidden items-center gap-6 text-sm text-white/85 sm:flex">
          <Link href="/" className="transition-colors hover:text-white">
            Home
          </Link>
        </nav>
        <div className="relative ml-auto">
          {label && (
            <>
              <button
                type="button"
                onClick={() => setMenuOpen((v) => !v)}
                className="flex items-center gap-2 rounded px-2 py-1 text-sm text-white/85 hover:text-white"
              >
                <span className="hidden sm:inline">{label}</span>
                <span className="inline-grid h-7 w-7 place-items-center rounded bg-(--color-accent)/25 text-xs font-medium uppercase">
                  {label.slice(0, 1)}
                </span>
              </button>
              {menuOpen && (
                <div className="absolute right-0 top-full mt-2 w-44 rounded border border-white/15 bg-(--color-surface) py-1 text-sm shadow-2xl">
                  <button
                    type="button"
                    onClick={logout}
                    className="block w-full px-3 py-2 text-left text-white/85 transition-colors hover:bg-white/5 hover:text-white"
                  >
                    Sign out
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </header>
  );
}
