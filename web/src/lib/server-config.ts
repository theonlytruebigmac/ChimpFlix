"use client";

// Module-level cache for the server-side config the player + Card
// component need. One fetch per page load, shared across every Card
// instance. Falls back to sensible defaults while loading so a
// freshly-rendered Card doesn't flash an incorrect badge before the
// config arrives.
//
// Promoting this to a React Context would be cleaner if more
// components grow to need it, but right now Card is the only consumer
// and a context provider would mean wrapping the whole tree just for
// one int.

import { useEffect, useState } from "react";
import { playState as playStateApi, type PlayStateConfig } from "./chimpflix-api";

const DEFAULT_RECENTLY_ADDED_DAYS = 14;

let cached: PlayStateConfig | null = null;
let inFlight: Promise<PlayStateConfig> | null = null;
const subscribers = new Set<() => void>();

function notify(): void {
  for (const fn of subscribers) fn();
}

function ensureFetch(): Promise<PlayStateConfig> {
  if (cached) return Promise.resolve(cached);
  if (inFlight) return inFlight;
  inFlight = playStateApi
    .config()
    .then((c) => {
      cached = c;
      inFlight = null;
      notify();
      return c;
    })
    .catch((e) => {
      inFlight = null;
      // Don't poison the cache — let the next caller retry. Card
      // falls back to the default in the meantime.
      throw e;
    });
  return inFlight;
}

/// Subscribe-style hook so multiple Cards can share the single
/// outstanding fetch and re-render together when it lands.
function useServerConfig(): PlayStateConfig | null {
  const [snap, setSnap] = useState<PlayStateConfig | null>(cached);
  useEffect(() => {
    const onChange = () => setSnap(cached);
    subscribers.add(onChange);
    if (!cached) {
      ensureFetch().catch(() => {});
    }
    return () => {
      subscribers.delete(onChange);
    };
  }, []);
  return snap;
}

/// "How many days does the Recently Added badge stay on a Card?"
/// Returns 14 (the historical hardcoded value) until the config
/// fetch resolves, then whatever the operator configured. Returns 0
/// when the operator has explicitly disabled the badge.
export function useRecentlyAddedDays(): number {
  const cfg = useServerConfig();
  if (cfg == null) return DEFAULT_RECENTLY_ADDED_DAYS;
  return Math.max(0, Math.min(365, cfg.recently_added_days));
}

const DEFAULT_PLAYED_THRESHOLD_PCT = 90;

/// Percentage (50–99) at which the server treats an item as
/// "effectively watched" for auto-scrobble. Returns 90 (the server
/// default) until the config fetch resolves. The Up-Next picker uses
/// this so a near-complete-but-not-explicitly-watched episode isn't
/// flagged as the next to play.
export function usePlayedThresholdPct(): number {
  const cfg = useServerConfig();
  if (cfg == null) return DEFAULT_PLAYED_THRESHOLD_PCT;
  return Math.max(50, Math.min(99, cfg.played_threshold_pct));
}
