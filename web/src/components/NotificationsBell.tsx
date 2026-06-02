"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type Notification,
} from "@/lib/chimpflix-api";

const POLL_INTERVAL_MS = 60_000;

/**
 * Bell icon + popover for the per-user notification inbox. Polls the
 * lightweight unread-count endpoint every minute so the badge stays
 * fresh without a websocket. Clicking the bell fetches the full list
 * lazily — most users never click, so we avoid that cost on page load.
 */
export function NotificationsBell() {
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<Notification[] | null>(null);
  const [loading, setLoading] = useState(false);
  // Snapshot of `Date.now()` taken at popover open. Used by row
  // renders to compute "X minutes ago" without calling Date.now()
  // during render (which is impure under strict mode).
  const [openedAtMs, setOpenedAtMs] = useState<number>(0);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function poll() {
      try {
        const { unread } = await authApi.notifications.unreadCount();
        if (!cancelled) setUnread(unread);
      } catch (e) {
        // 401 just means signed out — the parent guard already
        // handles that; silence everything else too. The badge will
        // stay at its last-known count.
        if (e instanceof ChimpFlixApiError && e.status === 401) {
          if (!cancelled) setUnread(0);
        }
      }
    }
    poll();
    const t = window.setInterval(poll, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(t);
    };
  }, []);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
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

  async function openAndLoad() {
    setOpen(true);
    setOpenedAtMs(Date.now());
    if (items === null) {
      setLoading(true);
      try {
        const res = await authApi.notifications.list();
        setItems(res.notifications);
        setUnread(res.unread);
      } catch {
        setItems([]);
      } finally {
        setLoading(false);
      }
    }
  }

  async function markAll() {
    try {
      await authApi.notifications.markAllRead();
      setItems((prev) =>
        prev?.map((n) => (n.read_at ? n : { ...n, read_at: Date.now() })) ??
        prev,
      );
      setUnread(0);
    } catch {
      /* swallow */
    }
  }

  async function clearAll() {
    // Optimistically empty the inbox; mark-read only dims rows, whereas
    // Clear removes them so the list doesn't grow without bound.
    const prev = items;
    setItems([]);
    setUnread(0);
    try {
      await authApi.notifications.clearAll();
    } catch {
      // Restore on failure so the user knows it didn't take.
      setItems(prev);
    }
  }

  async function markOne(id: number) {
    try {
      await authApi.notifications.markRead(id);
      setItems((prev) =>
        prev?.map((n) =>
          n.id === id && !n.read_at ? { ...n, read_at: Date.now() } : n,
        ) ?? prev,
      );
      setUnread((u) => Math.max(0, u - 1));
    } catch {
      /* swallow — a 404 just means already-read */
    }
  }

  return (
    <div ref={wrapRef} className="relative">
      <button
        type="button"
        onClick={() => (open ? setOpen(false) : openAndLoad())}
        aria-label={unread > 0 ? `Notifications (${unread} unread)` : "Notifications"}
        aria-expanded={open}
        className="relative flex h-8 w-8 items-center justify-center rounded-full text-white/80 transition-colors hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-(--color-accent) focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <BellIcon />
        {unread > 0 && (
          <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-(--color-accent) px-1 text-[10px] font-bold leading-none text-white">
            {unread > 99 ? "99+" : unread}
          </span>
        )}
      </button>

      {open && (
        <div
          role="menu"
          // On mobile (≤sm) snap the popover to ~90vw and right-align it
          // tightly so it fits a 360px viewport with the bell's right-2
          // padding. At sm+ it widens to a fixed 24rem (384px).
          className="fixed right-2 top-16 z-50 w-[calc(100vw-1rem)] max-w-sm overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm sm:absolute sm:right-0 sm:top-full sm:mt-2 sm:w-96"
        >
          <div className="flex items-center justify-between border-b border-white/10 px-3 py-2">
            <div className="text-xs font-semibold uppercase tracking-wider text-white/55">
              Notifications
            </div>
            <div className="flex items-center gap-3">
              {unread > 0 && (
                <button
                  type="button"
                  onClick={markAll}
                  className="rounded text-[11px] text-white/60 underline-offset-2 transition-colors hover:text-white hover:underline focus:outline-none focus-visible:text-white focus-visible:underline focus-visible:ring-1 focus-visible:ring-(--color-accent)"
                >
                  Mark all read
                </button>
              )}
              {(items?.length ?? 0) > 0 && (
                <button
                  type="button"
                  onClick={clearAll}
                  className="rounded text-[11px] text-white/60 underline-offset-2 transition-colors hover:text-white hover:underline focus:outline-none focus-visible:text-white focus-visible:underline focus-visible:ring-1 focus-visible:ring-(--color-accent)"
                >
                  Clear all
                </button>
              )}
            </div>
          </div>

          <div className="max-h-96 overflow-y-auto">
            {loading && (
              <p className="px-3 py-4 text-xs text-white/50">Loading…</p>
            )}
            {!loading && (items?.length ?? 0) === 0 && (
              <p className="px-3 py-6 text-center text-xs text-white/50">
                Nothing new.
              </p>
            )}
            {!loading &&
              items?.map((n) => (
                <NotificationRow
                  key={n.id}
                  notification={n}
                  nowMs={openedAtMs}
                  onMarkRead={() => markOne(n.id)}
                />
              ))}
          </div>
        </div>
      )}
    </div>
  );
}

function NotificationRow({
  notification,
  nowMs,
  onMarkRead,
}: {
  notification: Notification;
  /// Captured at popover-open time by the parent so we don't call
  /// the impure `Date.now()` during render.
  nowMs: number;
  onMarkRead: () => void;
}) {
  const unread = !notification.read_at;
  const { title, body, href } = render(notification);
  const ageMin = Math.max(1, Math.floor((nowMs - notification.created_at) / 60_000));

  const inner = (
    <div className="flex items-start gap-3 px-3 py-2.5">
      {unread ? (
        <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-(--color-accent)" />
      ) : (
        <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-transparent" />
      )}
      <div className="min-w-0 flex-1">
        <div className={`text-sm ${unread ? "text-white" : "text-white/70"}`}>
          {title}
        </div>
        <div className="mt-0.5 text-xs text-white/55">{body}</div>
        <div className="mt-0.5 text-[10px] uppercase tracking-wider text-white/35">
          {formatAge(ageMin)}
        </div>
      </div>
    </div>
  );

  if (href) {
    return (
      <Link
        href={href}
        onClick={onMarkRead}
        className="block transition-colors hover:bg-white/5 focus:outline-none focus-visible:bg-white/10"
      >
        {inner}
      </Link>
    );
  }
  return (
    <button
      type="button"
      onClick={onMarkRead}
      className="block w-full text-left transition-colors hover:bg-white/5 focus:outline-none focus-visible:bg-white/10"
    >
      {inner}
    </button>
  );
}

interface Rendered {
  title: string;
  body: string;
  /** Where to navigate when the user clicks the row. */
  href?: string;
}

function render(n: Notification): Rendered {
  let payload: Record<string, unknown> = {};
  try {
    payload = JSON.parse(n.payload_json) as Record<string, unknown>;
  } catch {
    /* leave empty */
  }
  switch (n.kind) {
    case "user.registered": {
      const username = String(payload.username ?? "");
      const display = String(payload.display_name ?? username);
      return {
        title: `${display} joined`,
        body: `@${username} accepted their invite. Grant library access if needed.`,
        href: "/settings/admin/users?tab=access",
      };
    }
    case "user.2fa.disabled": {
      const username = String(payload.username ?? "");
      return {
        title: `2FA disabled`,
        body: `@${username} turned off their two-factor.`,
        href: "/settings/admin/users",
      };
    }
    case "user.2fa.reset": {
      const actor = String(payload.actor_username ?? "");
      const target = String(payload.target_username ?? "");
      return {
        title: `2FA reset for @${target}`,
        body: `@${actor} reset their 2FA from the admin panel.`,
        href: "/settings/admin/users",
      };
    }
    case "content.new_movie": {
      // Batch summary payload carries `count` + `library_name`; the
      // singular payload carries `title` + `year` + `item_id`.
      if (typeof payload.count === "number") {
        const count = Number(payload.count);
        const lib = String(payload.library_name ?? "your library");
        return {
          title: `${count} new movies`,
          body: `${count} new movies were added to ${lib}.`,
          href: "/",
        };
      }
      const title = String(payload.title ?? "A new movie");
      const year = payload.year ? ` (${String(payload.year)})` : "";
      const itemId = payload.item_id;
      return {
        title: `New movie: ${title}${year}`,
        body: `${title}${year} was just added.`,
        href: itemId != null ? `/?title=${String(itemId)}` : "/",
      };
    }
    case "content.new_episode": {
      const showTitle = String(payload.show_title ?? "a show you watch");
      const showId = payload.show_id;
      const href = showId != null ? `/?title=${String(showId)}` : "/";
      // Batch summary payload carries `count`; the singular payload carries
      // `season_number` / `episode_number` / optional `episode_title`.
      if (typeof payload.count === "number") {
        const count = Number(payload.count);
        return {
          title: `${count} new episodes of ${showTitle}`,
          body: `${count} new episodes are ready to watch.`,
          href,
        };
      }
      const s = Number(payload.season_number ?? 0);
      const e = Number(payload.episode_number ?? 0);
      const code = `S${String(s).padStart(2, "0")}E${String(e).padStart(2, "0")}`;
      const epTitle = payload.episode_title ? ` — ${String(payload.episode_title)}` : "";
      return {
        title: `New episode of ${showTitle}`,
        body: `${code}${epTitle} is ready to watch.`,
        href,
      };
    }
    default:
      return { title: n.kind, body: "" };
  }
}

function formatAge(minutes: number): string {
  if (minutes < 60) return `${minutes}m ago`;
  if (minutes < 60 * 24) return `${Math.floor(minutes / 60)}h ago`;
  return `${Math.floor(minutes / (60 * 24))}d ago`;
}

function BellIcon() {
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
      <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
      <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
    </svg>
  );
}
