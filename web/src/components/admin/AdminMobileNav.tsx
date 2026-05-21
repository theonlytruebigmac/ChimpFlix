"use client";

import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { AdminNav } from "./AdminNav";

/**
 * Mobile-only hamburger that opens the admin nav as a slide-out drawer.
 * Hidden on screens at the `md` breakpoint and above, where the layout
 * shows the regular sticky sidebar instead. The drawer renders the
 * same <AdminNav /> inside so the two views stay in sync.
 */
export function AdminMobileNav() {
  const pathname = usePathname();
  const [open, setOpen] = useState(false);
  const [lastPathname, setLastPathname] = useState(pathname);

  // Close on route change — drawer items navigate by `<Link>`, so we
  // auto-dismiss when the path updates. Uses React's "adjust state
  // during render" pattern instead of a setState-in-effect so the close
  // happens in the same render as the route change (no flash of open
  // drawer at the new URL).
  if (pathname !== lastPathname) {
    setLastPathname(pathname);
    setOpen(false);
  }

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
        aria-label="Open admin navigation"
        aria-expanded={open}
        className="mb-3 inline-flex items-center gap-2 rounded-md border border-white/15 bg-white/5 px-3 py-2 text-sm text-white/85 transition-colors hover:bg-white/10 md:hidden"
      >
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
          <line x1="4" y1="7" x2="20" y2="7" />
          <line x1="4" y1="12" x2="20" y2="12" />
          <line x1="4" y1="17" x2="20" y2="17" />
        </svg>
        Admin sections
      </button>

      {open && (
        <div className="fixed inset-0 z-60 md:hidden">
          <button
            type="button"
            aria-label="Close menu"
            onClick={() => setOpen(false)}
            className="absolute inset-0 bg-black/70"
          />
          <aside
            role="dialog"
            aria-label="Admin navigation"
            aria-modal="true"
            className="relative flex h-full w-[85vw] max-w-xs flex-col bg-black shadow-2xl"
          >
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
              <span className="text-xs font-semibold uppercase tracking-wider text-white/50">
                Admin
              </span>
              <button
                type="button"
                onClick={() => setOpen(false)}
                aria-label="Close menu"
                className="flex h-9 w-9 items-center justify-center rounded-md text-white/70 hover:bg-white/10 hover:text-white"
              >
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
              </button>
            </div>
            <div className="flex-1 overflow-y-auto px-3 py-3">
              <AdminNav />
            </div>
          </aside>
        </div>
      )}
    </>
  );
}
