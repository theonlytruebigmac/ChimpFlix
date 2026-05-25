// Hover-time session pre-warm cache.
//
// On hover of the modal Play button we POST `/stream/prewarm` so
// ffmpeg starts encoding the first few HLS segments before the user
// has clicked anything. When the user does click and the player
// mounts, it asks `consumePrewarm` for a session matching its
// resolved (mediaFileId, startPositionMs). A hit skips the player's
// own POST and adopts the already-encoding session directly —
// shaving the 1-3 s ffmpeg-warmup gap off the perceived play
// latency.
//
// Guardrails:
//   * Single in-flight prewarm at a time. A new hover for a
//     different ratingKey aborts the previous (DELETEs the session).
//   * Time-to-live of 60 s. After that the cache entry is treated as
//     stale and discarded — the idle reaper on the server cleans up
//     the underlying session.
//   * On unmount / page hide the modal cancels its prewarm to keep
//     orphan sessions from piling up under indecisive hovers.
//
// The match contract is intentionally narrow: same media_file_id and
// resume position within 2 s. Anything else (user picked a different
// audio track, switched versions, or scrubbed) misses the cache and
// the player falls through to its normal `createSession` flow.

import {
  ChimpFlixApiError,
  stream as streamApi,
  type ClientCapabilities,
  type SessionInfo,
} from "@/lib/chimpflix-api";
import { devWarn } from "@/lib/dev-log";

const TTL_MS = 60_000;
/// Resume position tolerance when matching a cached prewarm against
/// a player mount. Resume positions in the wild bounce a few hundred
/// ms between resume reads (the watch page reads from the DB at SSR
/// time; the player rereads later). 2 s lets those land while still
/// catching genuine user scrubs.
const MATCH_TOLERANCE_MS = 2000;

interface Cached {
  ratingKey: string;
  session: SessionInfo;
  createdAtMs: number;
  consumed: boolean;
}

let current: Cached | null = null;
/// Tracks the in-flight prewarm so a rapid second hover for the
/// same ratingKey doesn't double up on the request.
let inflight: Promise<void> | null = null;

/// Kick off a prewarm for `ratingKey`. Idempotent for the same key
/// — repeated calls during the hover window are folded into one
/// in-flight request. A different key cancels and replaces.
///
/// `clientCaps` should be the same `detectClientCapabilities()`
/// result the player will send so the server's direct/transcode
/// decision lands the same way.
export function prewarmFor(
  ratingKey: string,
  clientCaps: ClientCapabilities,
  audioNormalize: boolean,
): void {
  // Already warmed for this exact key? No-op. Match exists if the
  // entry isn't consumed and is still within TTL — otherwise treat
  // as a miss and re-warm.
  if (
    current &&
    current.ratingKey === ratingKey &&
    !current.consumed &&
    Date.now() - current.createdAtMs < TTL_MS
  ) {
    return;
  }
  if (inflight) return;

  // Different key in cache: cancel and clear before the new request
  // races. The DELETE is best-effort; we don't wait on it.
  if (current && current.ratingKey !== ratingKey) {
    void cancelPrewarm();
  }

  inflight = (async () => {
    try {
      const resp = await streamApi.prewarmSession({
        rating_key: ratingKey,
        client: clientCaps,
        audio_normalize: audioNormalize ? true : undefined,
      });
      current = {
        ratingKey,
        session: resp.session,
        createdAtMs: Date.now(),
        consumed: false,
      };
    } catch (e) {
      // Permission errors (401/404) or unsupported routes shouldn't
      // poison subsequent hovers. We just log and forget — the
      // player's own createSession path is the source of truth.
      if (!(e instanceof ChimpFlixApiError)) {
        devWarn("[prewarm] failed:", e);
      }
      current = null;
    } finally {
      inflight = null;
    }
  })();
}

/// Hand the cached session over to the player if it matches the
/// player's resolved (mediaFileId, startPositionMs). The cache entry
/// is cleared on hit so subsequent mounts don't double-adopt the
/// same session.
export function consumePrewarm(
  mediaFileId: number,
  startPositionMs: number,
): SessionInfo | null {
  if (!current || current.consumed) return null;
  if (Date.now() - current.createdAtMs >= TTL_MS) {
    void cancelPrewarm();
    return null;
  }
  if (current.session.media_file_id !== mediaFileId) return null;
  const cached = current.session.start_position_ms;
  if (Math.abs(cached - startPositionMs) > MATCH_TOLERANCE_MS) {
    return null;
  }
  current.consumed = true;
  const session = current.session;
  current = null;
  return session;
}

/// Cancel any unconsumed prewarm. Fires DELETE on the backing
/// session if one was returned; clears the cache either way. Safe
/// to call when nothing is cached.
export async function cancelPrewarm(): Promise<void> {
  const c = current;
  current = null;
  if (!c || c.consumed) return;
  if (c.session.mode !== "transcode") return;
  try {
    await streamApi.deleteSession(c.session.id);
  } catch {
    // The idle reaper will mop up if our DELETE didn't make it.
  }
}
