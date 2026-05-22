"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
  plex,
  ChimpFlixApiError,
  type PlexStartInput,
  type PlexPollResult,
  type AuthResponse,
} from "@/lib/chimpflix-api";

/**
 * "Sign in with Plex" / "Link Plex" / "Create account with Plex" — one
 * component, three intents driven by the prop the parent passes. On
 * click we:
 *
 *   1. POST `/auth/plex/start` to mint a server-side PIN handle.
 *   2. `window.open(auth_url, "_blank")` so the user authorizes on
 *      plex.tv in a new tab without losing context here.
 *   3. Poll `/auth/plex/poll` every 2s until a terminal result.
 *
 * Terminal results bubble back to the parent via `onSuccess`,
 * `onNotLinked`, and `onError`. The parent decides where to send the
 * user next (`/` after a login, settings refresh after a link, etc.).
 *
 * Polling stops on unmount + on terminal status — there's no exposure
 * window where a closed page keeps hammering the API.
 */
export function PlexSignInButton({
  intent,
  label,
  onSuccess,
  onLinked,
  onNotLinked,
  onError,
  disabled,
}: {
  intent: PlexStartInput;
  /** Override the button label. Defaults to a sensible string per intent. */
  label?: string;
  /** Called when login / signup completes with the new session payload. */
  onSuccess?: (resp: AuthResponse) => void;
  /** Called when the `link` intent successfully attaches a Plex identity. */
  onLinked?: () => void;
  /** Called for `login` intent when the Plex identity isn't bound to a local user. */
  onNotLinked?: (plexUsername: string) => void;
  onError?: (message: string) => void;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState<"idle" | "authorizing" | "polling">("idle");
  const pollTimer = useRef<number | null>(null);
  const aliveRef = useRef(true);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      if (pollTimer.current) window.clearInterval(pollTimer.current);
    };
  }, []);

  const stopPolling = useCallback(() => {
    if (pollTimer.current) {
      window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
    setPhase("idle");
    setBusy(false);
  }, []);

  const finish = useCallback(
    (cb: () => void) => {
      stopPolling();
      cb();
    },
    [stopPolling],
  );

  async function onClick() {
    if (busy || disabled) return;
    setBusy(true);
    setPhase("authorizing");
    // Open a placeholder tab synchronously inside the click handler.
    // Two reasons:
    //   1. The user-gesture context is preserved, so popup blockers
    //      don't trip when we navigate the tab later (a `window.open`
    //      called from inside an awaited callback has lost the gesture
    //      and is much more likely to get blocked).
    //   2. The tab appears instantly. The ~1 s it takes for our backend
    //      to round-trip Plex's `/pins` endpoint is hidden behind a
    //      blank tab the user sees pop up immediately — no perceived
    //      "click did nothing" lag.
    // If `window.open` returns null (true popup blocker, not the
    // `noopener` false-positive we used to hit), we fall back to a
    // same-tab redirect after the URL is ready.
    const placeholder = window.open("about:blank", "_blank");
    try {
      const start = await plex.start(intent);
      if (placeholder && !placeholder.closed) {
        placeholder.location.href = start.auth_url;
      } else {
        // Popup actually blocked. Keep the user in flow by navigating
        // the current tab; on return, polling resumes the link state.
        window.location.href = start.auth_url;
      }
      setPhase("polling");
      const tick = async () => {
        if (!aliveRef.current) return;
        try {
          const result = await plex.poll(start.pin_handle);
          handlePollResult(result);
        } catch (e) {
          finish(() => onError?.(parseError(e)));
        }
      };
      // First tick promptly, then on a 2s interval. The user typically
      // takes 5-10s on plex.tv; polling sooner makes the "Linked!"
      // transition feel instant when they tab back.
      tick();
      pollTimer.current = window.setInterval(tick, 2000);
    } catch (e) {
      // Close the placeholder tab we opened proactively so the user
      // isn't left with a stray blank tab on a failure path.
      if (placeholder && !placeholder.closed) placeholder.close();
      finish(() => onError?.(parseError(e)));
    }
  }

  function handlePollResult(result: PlexPollResult) {
    if ("user" in result) {
      finish(() => onSuccess?.(result));
      return;
    }
    switch (result.status) {
      case "pending":
        // Keep polling. setBusy stays true.
        return;
      case "expired":
        finish(() =>
          onError?.("The Plex authorization timed out. Click \"Sign in with Plex\" to try again."),
        );
        return;
      case "unknown_handle":
        // Server forgot about us — most likely a restart. Treat as expired.
        finish(() => onError?.("The authorization was lost. Try again."));
        return;
      case "not_linked":
        finish(() => onNotLinked?.(result.plex_username));
        return;
      case "linked":
        finish(() => onLinked?.());
        return;
    }
  }

  const fallbackLabel =
    intent.intent === "link"
      ? "Link Plex account"
      : intent.intent === "signup"
        ? "Create account with Plex"
        : "Sign in with Plex";

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy || disabled}
      className="flex w-full items-center justify-center gap-2 rounded border border-white/15 bg-white/5 px-3 py-2.5 text-sm font-medium text-white transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
    >
      <PlexGlyph />
      {phase === "authorizing"
        ? "Opening Plex…"
        : phase === "polling"
          ? "Waiting for Plex approval…"
          : (label ?? fallbackLabel)}
    </button>
  );
}

function PlexGlyph() {
  // Stylized Plex chevron, single colour so it picks up `currentColor`
  // from the button text and stays readable across themes. Bundled
  // inline rather than added to /public — it's tiny and avoids a
  // network request on the login screen.
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M6 2h6l6 10-6 10H6l6-10z" />
    </svg>
  );
}

function parseError(e: unknown): string {
  if (e instanceof ChimpFlixApiError) {
    try {
      const parsed = JSON.parse(e.body) as {
        error?: { message?: string } | string;
      };
      if (parsed.error && typeof parsed.error === "object" && parsed.error.message) {
        return parsed.error.message;
      }
      if (typeof parsed.error === "string") return parsed.error;
    } catch {
      /* fall through */
    }
    return `Error ${e.status}`;
  }
  return e instanceof Error ? e.message : "Network error";
}
