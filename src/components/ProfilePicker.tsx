"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import type { HomeUser } from "@/lib/plex";

export function ProfilePicker({ users }: { users: HomeUser[] }) {
  const router = useRouter();
  const [pinUser, setPinUser] = useState<HomeUser | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  // Warm the home route's RSC payload + bundles so the post-switch transition
  // feels instant. The prefetch happens with the *current* (admin) cookie,
  // so the data isn't reused — but the JS bundles, fonts, and route manifest
  // load up front, which is the bulk of the wait on a cold visit.
  useEffect(() => {
    router.prefetch("/");
  }, [router]);

  async function selectProfile(user: HomeUser, pin?: string) {
    setSubmitting(true);
    setError(null);
    try {
      const res = await fetch("/api/auth/switch", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ userId: user.id, pin }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        if (res.status === 401) {
          setError("Wrong PIN — try again.");
        } else {
          setError(data.error ?? `Failed (${res.status})`);
        }
        setSubmitting(false);
        return;
      }
      // Soft-nav to /: keeps the JS bundle warm and avoids a full reload.
      // refresh() invalidates the RSC cache so the home page re-renders
      // server-side with the new cf_token cookie. Also drop the cached
      // /api/auth/me identity so TopNav doesn't briefly show the previous
      // profile's avatar after the swap.
      try {
        window.sessionStorage.removeItem("cf_auth_me");
      } catch {
        // ignore — privacy mode etc.
      }
      router.replace("/");
      router.refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setSubmitting(false);
    }
  }

  function onSelect(user: HomeUser) {
    if (user.hasPassword || user.protected) {
      setPinUser(user);
      setError(null);
    } else {
      selectProfile(user);
    }
  }

  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-6 py-16">
      <h1 className="mb-12 text-4xl font-medium tracking-tight text-white/90 sm:text-5xl">
        Who&apos;s watching?
      </h1>

      <ul className="flex flex-wrap items-start justify-center gap-x-8 gap-y-10 sm:gap-x-12">
        {users.map((u) => (
          <li key={u.id}>
            <button
              type="button"
              onClick={() => onSelect(u)}
              disabled={submitting}
              className="group flex flex-col items-center gap-3 disabled:opacity-50"
            >
              <ProfileAvatar user={u} />
              <div className="flex items-center gap-1.5 text-base font-medium text-white/65 transition-colors group-hover:text-white">
                {u.title}
                {(u.hasPassword || u.protected) && <LockIcon />}
              </div>
            </button>
          </li>
        ))}
      </ul>

      {pinUser && (
        <PinModal
          user={pinUser}
          submitting={submitting}
          error={error}
          onCancel={() => {
            setPinUser(null);
            setError(null);
          }}
          onSubmit={(pin) => selectProfile(pinUser, pin)}
        />
      )}

      {error && !pinUser && (
        <p className="mt-6 text-sm text-(--color-accent)">{error}</p>
      )}
    </div>
  );
}

function ProfileAvatar({ user }: { user: HomeUser }) {
  const initial = (user.title || user.username || "?").charAt(0).toUpperCase();
  return user.thumb ? (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={user.thumb}
      alt=""
      className="h-32 w-32 rounded-md object-cover transition-all group-hover:scale-105 group-hover:ring-4 group-hover:ring-white sm:h-36 sm:w-36"
    />
  ) : (
    <div className="flex h-32 w-32 items-center justify-center rounded-md bg-white/10 text-4xl font-bold text-white/85 transition-all group-hover:scale-105 group-hover:bg-white/15 group-hover:ring-4 group-hover:ring-white sm:h-36 sm:w-36">
      {initial}
    </div>
  );
}

function PinModal({
  user,
  submitting,
  error,
  onCancel,
  onSubmit,
}: {
  user: HomeUser;
  submitting: boolean;
  error: string | null;
  onCancel: () => void;
  onSubmit: (pin: string) => void;
}) {
  const [pin, setPin] = useState("");

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onCancel();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (pin.length === 4) onSubmit(pin);
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 px-6"
      onClick={onCancel}
    >
      <form
        onSubmit={handleSubmit}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md rounded-md border border-white/10 bg-(--color-surface) p-8"
      >
        <h2 className="mb-2 text-2xl font-semibold text-white">
          Enter PIN for {user.title}
        </h2>
        <p className="mb-6 text-sm text-white/60">
          A 4-digit PIN was set up on plex.tv for this profile.
        </p>
        <input
          autoFocus
          inputMode="numeric"
          pattern="[0-9]*"
          maxLength={4}
          value={pin}
          onChange={(e) => setPin(e.target.value.replace(/\D/g, ""))}
          aria-label="PIN"
          className="block w-full rounded-md border border-white/20 bg-black/40 px-4 py-3 text-center font-mono text-2xl tracking-[0.6em] text-white focus:border-white focus:outline-none"
          placeholder="••••"
        />
        {error && (
          <p className="mt-3 text-sm text-(--color-accent)">{error}</p>
        )}
        <div className="mt-6 flex gap-3">
          <button
            type="submit"
            disabled={pin.length !== 4 || submitting}
            className="flex-1 rounded-md bg-(--color-accent) py-2.5 text-sm font-semibold text-white transition-colors hover:bg-(--color-accent-hover) disabled:opacity-60"
          >
            {submitting ? "Switching…" : "Continue"}
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md border border-white/30 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:border-white"
          >
            Cancel
          </button>
        </div>
      </form>
    </div>
  );
}

function LockIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="4" y="11" width="16" height="10" rx="2" />
      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
    </svg>
  );
}
