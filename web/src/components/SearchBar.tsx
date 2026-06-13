"use client";

import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";

export function SearchBar() {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const onSearchPage = pathname === "/search";

  const initialQuery = onSearchPage ? (searchParams.get("q") ?? "") : "";
  const [open, setOpen] = useState(initialQuery.length > 0);
  const [query, setQuery] = useState(initialQuery);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<number | null>(null);
  // True only while the search input is focused (the user is actively
  // typing). The debounced navigation is gated on this so that LEAVING
  // the search page — e.g. clicking the "Home" nav link, which flips
  // `onSearchPage` and re-runs the effect below while `query` still holds
  // the old text — can't fire a `router.push("/search?q=…")` and bounce
  // the user straight back to the search page.
  const focusedRef = useRef(false);

  // Keep local query in sync when /search?q=... changes (e.g. browser
  // back). The setState is the "URL is the source of truth, mirror
  // it into local state" pattern — the documented exception to the
  // set-state-in-effect rule.
  useEffect(() => {
    if (onSearchPage) {
      const q = searchParams.get("q") ?? "";
      /* eslint-disable react-hooks/set-state-in-effect */
      setQuery(q);
      setOpen(q.length > 0 || open);
      /* eslint-enable react-hooks/set-state-in-effect */
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams, onSearchPage]);

  // Debounce URL updates as the user types.
  useEffect(() => {
    if (!open) return;
    if (debounceRef.current) window.clearTimeout(debounceRef.current);

    const trimmed = query.trim();
    debounceRef.current = window.setTimeout(() => {
      // Only drive navigation while the user is actually typing in the
      // (focused) box. Without this, the effect re-running because the
      // path changed (e.g. navigating Home) would push back to /search.
      if (!focusedRef.current) return;
      if (!trimmed) {
        if (onSearchPage) router.replace("/search");
        return;
      }
      const url = `/search?q=${encodeURIComponent(trimmed)}`;
      if (onSearchPage) router.replace(url, { scroll: false });
      else router.push(url, { scroll: false });
    }, 250);

    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
    };
  }, [query, open, onSearchPage, router]);

  const close = useCallback(() => {
    setOpen(false);
    setQuery("");
    if (onSearchPage) router.push("/");
  }, [onSearchPage, router]);

  const openInput = useCallback(() => {
    setOpen(true);
    // Wait for the input to render before focusing.
    window.setTimeout(() => inputRef.current?.focus(), 0);
  }, []);

  // Global "/" shortcut focuses the search input — standard convention. Only
  // fires when the user isn't already typing somewhere else.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "/") return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      e.preventDefault();
      openInput();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openInput]);

  function onKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      e.currentTarget.blur();
      close();
    }
  }

  return (
    <div className="flex items-center">
      {open ? (
        <div className="flex items-center gap-2 rounded border border-white/40 bg-black/60 px-2.5 py-1.5">
          <SearchIcon />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            onFocus={() => {
              focusedRef.current = true;
            }}
            onBlur={() => {
              focusedRef.current = false;
            }}
            placeholder="Search titles"
            className="w-48 bg-transparent text-sm text-white placeholder:text-white/50 focus:outline-none sm:w-64"
            aria-label="Search"
          />
          <button
            type="button"
            onClick={close}
            aria-label="Close search"
            className="text-white/70 transition-colors hover:text-white"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              aria-hidden
            >
              <line x1="6" y1="6" x2="18" y2="18" />
              <line x1="18" y1="6" x2="6" y2="18" />
            </svg>
          </button>
        </div>
      ) : (
        <button
          type="button"
          onClick={openInput}
          aria-label="Open search"
          className="text-white/85 transition-colors hover:text-white"
        >
          <SearchIcon />
        </button>
      )}
    </div>
  );
}

function SearchIcon() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="11" cy="11" r="7" />
      <line x1="20" y1="20" x2="16.65" y2="16.65" />
    </svg>
  );
}
