/// Google Cast (Chromecast) + Apple AirPlay glue.
///
/// Two protocols, one button. We surface a single "Cast" affordance
/// in the player; on iOS it routes through AirPlay (free, native to
/// HLS via `webkitShowPlaybackTargetPicker`), on Chromium browsers
/// it routes through the Cast Web Sender SDK.
///
/// **iOS reality check:** Apple WebKit does NOT expose the Cast
/// Web Sender API. There is no SDK-based path to Chromecast from
/// iOS Safari or any iOS PWA — third-party browsers on iOS all use
/// WebKit and the same restriction applies. AirPlay is the only
/// option on that platform.
///
/// **Cast auth model:** the Cast receiver fetches the manifest +
/// segments directly with no cookie jar. We mint a short-lived HMAC
/// token via `cast.sign()` and append `?ct=<token>` to the URLs the
/// receiver loads. See [`crates/server/src/auth/cast_token.rs`].
"use client";

import { useEffect, useState, type RefObject } from "react";
import { devError, devWarn } from "@/lib/dev-log";

/// Cast receiver application ID.
///
/// `CC1AD845` is Google's stock Default Media Receiver — it can play a
/// single direct-play file URL (the token rides in the query string and
/// there are no sub-requests), but it CANNOT play our HLS transcode
/// streams: the master playlist references variants/segments with bare
/// relative URLs, so the `?ct=` token is dropped on every sub-request
/// and the server 401s them.
///
/// Set `NEXT_PUBLIC_CAST_RECEIVER_APP_ID` (baked at build time) to the
/// custom receiver registered in the Google Cast SDK Developer Console
/// — point that registration at `https://<origin>/cast/receiver.html`,
/// which re-appends the token to every sub-request. We fall back to the
/// stock receiver when the var is unset so casting still works for
/// direct-play before the operator registers a custom receiver.
const RECEIVER_APP_ID =
  process.env.NEXT_PUBLIC_CAST_RECEIVER_APP_ID || "CC1AD845";

/// URL the Cast SDK is loaded from. Google updates this script in
/// place; we pin to v1 framework loader so the API surface stays
/// stable across SDK rolls.
const CAST_SDK_URL =
  "https://www.gstatic.com/cv/js/sender/v1/cast_sender.js?loadCastFramework=1";

// The Cast SDK installs deep `chrome.cast.*` / `cast.framework.*`
// globals. We don't pull in the official `@types/chromecast-caf-sender`
// dep just to read a handful of fields — narrowing the surface to
// `unknown` casts here keeps the bundle lean.
type CastWindow = Window & {
  chrome?: { cast?: unknown };
  cast?: { framework?: unknown };
  __onGCastApiAvailable?: (available: boolean) => void;
};

let castSdkLoad: Promise<boolean> | null = null;

/// Read the Cast framework off the window once the SDK has installed it.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function castFramework(): any | null {
  const w = window as CastWindow;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return (w.cast as any)?.framework ?? null;
}

/// Point the Cast context at our receiver app. Safe to call more than
/// once — the SDK just re-applies the options — so both the happy-path
/// loader and the slow-load recovery in `useCastState` can call it.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function configureCastContext(framework: any): boolean {
  try {
    framework.CastContext.getInstance().setOptions({
      receiverApplicationId: RECEIVER_APP_ID,
      // ORIGIN_SCOPED lets the SDK silently re-attach to a live session
      // if the user navigates between pages while a cast is active.
      autoJoinPolicy: framework.AutoJoinPolicy.ORIGIN_SCOPED,
    });
    return true;
  } catch (e) {
    devError("[cast] setOptions failed", e);
    return false;
  }
}

/// Lazy-load the Cast Web Sender SDK. Returns true if the SDK loaded
/// and a receiver-discoverable browser is running, false otherwise.
/// Memoised — calling twice doesn't double-inject the script.
export function loadCastSdk(): Promise<boolean> {
  if (typeof window === "undefined") return Promise.resolve(false);
  const w = window as CastWindow;
  if (castSdkLoad) return castSdkLoad;
  castSdkLoad = new Promise<boolean>((resolve) => {
    // Hard ceiling on how long we'll wait for the SDK to phone home.
    // On Android Chrome installed as a standalone PWA, the SDK script
    // *loads* (no onerror) but never calls __onGCastApiAvailable — the
    // system Cast media-router IPC isn't proxied through to standalone
    // web apps. Without a timeout the promise hangs and `cast.available`
    // never flips, with no log line to tell the operator why.
    //
    // 15s (was 8s): on a cold mobile load the chained framework script
    // from gstatic can take >8s to initialise and fire the callback. The
    // old budget routinely lost that race on phones and resolved
    // "unavailable", and because this promise is memoised the cast button
    // then stayed hidden for the whole page even after the SDK finished.
    // `useCastState` now also polls for the framework as a backstop, but a
    // roomier timeout keeps the happy path resolving true.
    const timeoutMs = 15000;
    let resolved = false;
    const finalize = (ok: boolean, reason: string) => {
      if (resolved) return;
      resolved = true;
      if (!ok) devWarn(`[cast] not available: ${reason}`);
      resolve(ok);
    };
    const timer = window.setTimeout(() => {
      finalize(
        false,
        "SDK timeout — __onGCastApiAvailable never fired. Likely cause: Android standalone PWA (Cast IPC not proxied) or browser without Cast support.",
      );
    }, timeoutMs);
    w.__onGCastApiAvailable = (available: boolean) => {
      window.clearTimeout(timer);
      if (!available) {
        // Most common reasons we land here in production: non-Chromium
        // browser (Firefox/Safari), Android standalone-PWA stripping
        // Cast IPC, or `chrome://flags` Cast disabled.
        finalize(false, "__onGCastApiAvailable(false)");
        return;
      }
      const framework = castFramework();
      if (!framework) {
        finalize(false, "SDK loaded but cast.framework is missing");
        return;
      }
      configureCastContext(framework);
      finalize(true, "ready");
    };
    const existing = document.querySelector<HTMLScriptElement>(
      `script[src="${CAST_SDK_URL}"]`,
    );
    if (existing) {
      // Script already in the DOM — the callback above will fire
      // when the SDK signals ready. If the SDK has already loaded
      // before we wired the callback, `chrome.cast` is set; we can
      // resolve immediately.
      if (w.chrome?.cast) finalize(true, "ready (cached)");
      return;
    }
    const script = document.createElement("script");
    script.src = CAST_SDK_URL;
    script.async = true;
    script.onerror = () => finalize(false, "sender SDK failed to load");
    document.head.appendChild(script);
  });
  return castSdkLoad;
}

/// Inspector probe — run `__cf_castDebug()` in DevTools to dump SDK
/// state without waiting on the React component tree. Useful when the
/// toolbar button is hidden and we want to confirm whether the SDK
/// actually loaded.
if (typeof window !== "undefined") {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).__cf_castDebug = () => {
    const w = window as CastWindow;
    const scriptInDom = !!document.querySelector(
      `script[src="${CAST_SDK_URL}"]`,
    );
    return {
      scriptInDom,
      hasChromeCast: !!w.chrome?.cast,
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      hasFramework: !!(w.cast as any)?.framework,
      displayMode: window.matchMedia("(display-mode: standalone)").matches
        ? "standalone"
        : window.matchMedia("(display-mode: minimal-ui)").matches
          ? "minimal-ui"
          : "browser",
      secureContext: window.isSecureContext,
      userAgent: navigator.userAgent,
    };
  };
}

/// Cast session state surfaced to the React tree. `available` flips
/// true once the Cast SDK has loaded — the toolbar button should
/// appear from that point so the affordance is discoverable even
/// before a receiver wakes up on the LAN. `hasDevices` narrows that
/// to "a receiver is actually on the network right now" so the
/// tooltip can read "Cast to device" vs "No cast devices found".
/// `connected` is true while a cast session is live (media may not
/// have started yet — see `mediaInfo`).
export interface CastState {
  available: boolean;
  hasDevices: boolean;
  connected: boolean;
  deviceName: string | null;
}

/// Subscribe to Cast SDK state. Returns a CastState that updates as
/// the SDK reports availability + session lifecycle changes.
export function useCastState(): CastState {
  const [state, setState] = useState<CastState>({
    available: false,
    hasDevices: false,
    connected: false,
    deviceName: null,
  });
  useEffect(() => {
    let cancelled = false;
    let wired = false;
    let pollTimer: number | null = null;
    const cleanups: Array<() => void> = [];

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const wireUp = (framework: any) => {
      if (wired || cancelled) return;
      wired = true;
      configureCastContext(framework);
      const ctx = framework.CastContext.getInstance();
      const sync = () => {
        if (cancelled) return;
        const session = ctx.getCurrentSession();
        const castState = ctx.getCastState();
        setState({
          // Framework is live — surface the button so the affordance is
          // discoverable even before a receiver wakes up. The picker
          // handles the empty case with "No devices found".
          available: true,
          // `castState`: NO_DEVICES_AVAILABLE / NOT_CONNECTED / CONNECTING
          // / CONNECTED. Anything but "no devices" means one is reachable.
          hasDevices: castState !== framework.CastState.NO_DEVICES_AVAILABLE,
          connected: castState === framework.CastState.CONNECTED,
          deviceName: session?.getCastDevice?.()?.friendlyName ?? null,
        });
      };
      sync();
      const onChange = () => sync();
      const evt = framework.CastContextEventType;
      ctx.addEventListener(evt.CAST_STATE_CHANGED, onChange);
      ctx.addEventListener(evt.SESSION_STATE_CHANGED, onChange);
      cleanups.push(() => {
        ctx.removeEventListener(evt.CAST_STATE_CHANGED, onChange);
        ctx.removeEventListener(evt.SESSION_STATE_CHANGED, onChange);
      });
    };

    const tryWire = (): boolean => {
      const framework = castFramework();
      if (framework) {
        wireUp(framework);
        return true;
      }
      return false;
    };

    (async () => {
      const loaded = await loadCastSdk();
      if (cancelled) return;
      if (loaded) {
        tryWire();
        return;
      }
      // loadCastSdk reported unavailable. In a standalone PWA that's real:
      // Android doesn't proxy the Cast media-router IPC into installed web
      // apps, so the framework exists but the picker can't enumerate
      // devices — keep the button hidden rather than show a dead one. In a
      // normal browser tab the framework can still finish initialising
      // after loadCastSdk's load-timeout already resolved false on a slow
      // connection, so poll briefly and self-heal instead of needing a
      // page reload.
      const standalone =
        window.matchMedia("(display-mode: standalone)").matches ||
        window.matchMedia("(display-mode: minimal-ui)").matches;
      if (standalone) return;
      if (tryWire()) return;
      let tries = 0;
      pollTimer = window.setInterval(() => {
        if (cancelled || wired || tries++ >= 40 || tryWire()) {
          if (pollTimer != null) {
            window.clearInterval(pollTimer);
            pollTimer = null;
          }
        }
      }, 500);
    })();

    return () => {
      cancelled = true;
      if (pollTimer != null) window.clearInterval(pollTimer);
      cleanups.forEach((c) => c());
    };
  }, []);
  return state;
}

/// Media info handed to the receiver when starting/loading a cast
/// session. All fields are optional except `url` and `contentType`.
export interface CastMediaPayload {
  url: string;
  contentType: string;
  title?: string;
  subtitle?: string;
  posterUrl?: string;
  /// Initial playback position in seconds. Pass to seek the
  /// receiver to where the user was watching locally.
  startTimeS?: number;
  /// Duration in seconds — lets the receiver paint a complete
  /// progress bar before the first manifest fetch returns.
  durationS?: number;
}

/// Outcome of a [`requestCastSession`] attempt. Lets the caller stay
/// silent on a normal user cancel while surfacing actionable reasons
/// ("no devices on the network", "couldn't reach the receiver") — the
/// difference between a Cast button that "does nothing" and one that
/// tells you why.
export type CastSessionResult = "ok" | "cancel" | "no-devices" | "error";

/// Open the Cast device picker and establish a session.
///
/// MUST be called directly from a user-gesture handler BEFORE any
/// `await` of a network request. `requestSession()` (which opens the
/// picker) needs transient user activation; an intervening awaited fetch
/// — e.g. minting the cast token via `/cast/sign` — consumes that
/// activation, after which the picker silently fails to open (most
/// visibly on mobile Chrome: "tap does nothing"). So the click handler
/// calls THIS first, then mints the token + loads the media.
export async function requestCastSession(): Promise<CastSessionResult> {
  const w = window as CastWindow;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const framework = (w.cast as any)?.framework;
  if (!framework) {
    devWarn("[cast] SDK not loaded; cannot start session");
    return "error";
  }
  const ctx = framework.CastContext.getInstance();
  if (ctx.getCurrentSession()) return "ok";
  try {
    // requestSession resolves once a session is live OR rejects with a
    // chrome.cast.Error whose `.code` tells us why: "cancel" (user
    // dismissed the picker), "receiver_unavailable" (no devices the
    // sender can see — the common flaky-mDNS / segmented-LAN case),
    // "timeout", etc.
    await ctx.requestSession();
  } catch (e) {
    devWarn("[cast] requestSession failed/cancelled", e);
    // The SDK throws either a bare string code or an object with `.code`.
    const code =
      typeof e === "string"
        ? e
        : // eslint-disable-next-line @typescript-eslint/no-explicit-any
          ((e as any)?.code as string | undefined);
    if (code === "cancel") return "cancel";
    if (code === "receiver_unavailable") return "no-devices";
    return "error";
  }
  return ctx.getCurrentSession() ? "ok" : "cancel";
}

/// Load media onto the CURRENT cast session. Call after
/// [`requestCastSession`] has established one — the session already
/// exists, so awaiting network work (the token mint) before this is
/// fine; no user gesture is needed any more.
export async function loadCastMedia(media: CastMediaPayload): Promise<boolean> {
  const w = window as CastWindow;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const chromeCast = (w.chrome as any)?.cast;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const framework = (w.cast as any)?.framework;
  if (!chromeCast || !framework) return false;
  const session = framework.CastContext.getInstance().getCurrentSession();
  if (!session) return false;
  try {
    const mediaInfo = new chromeCast.media.MediaInfo(media.url, media.contentType);
    mediaInfo.streamType = chromeCast.media.StreamType.BUFFERED;
    if (media.durationS != null) mediaInfo.duration = media.durationS;
    if (media.title || media.subtitle || media.posterUrl) {
      const metadata = new chromeCast.media.GenericMediaMetadata();
      if (media.title) metadata.title = media.title;
      if (media.subtitle) metadata.subtitle = media.subtitle;
      if (media.posterUrl) {
        metadata.images = [new chromeCast.Image(media.posterUrl)];
      }
      mediaInfo.metadata = metadata;
    }
    const request = new chromeCast.media.LoadRequest(mediaInfo);
    if (media.startTimeS != null) request.currentTime = media.startTimeS;
    await session.loadMedia(request);
    return true;
  } catch (e) {
    devWarn("[cast] loadMedia failed", e);
    return false;
  }
}

/// Snapshot of the casting receiver's playback, as the sender sees it.
/// `currentTimeS` is the position within the media the receiver loaded
/// (for our HLS that's media-time = source-time minus the transcode
/// fast-seek offset; for direct play it's absolute source-time). The
/// caller maps it back to source-time before persisting.
export interface RemotePlaybackState {
  currentTimeS: number;
  durationS: number;
  isPaused: boolean;
  isMediaLoaded: boolean;
}

/// Observe the currently-casting receiver's playback. `onChange` fires
/// whenever the position or paused/loaded state changes (the Cast SDK
/// emits CURRENT_TIME_CHANGED roughly once a second). Returns an
/// unsubscribe; a no-op if the SDK isn't loaded. Used by the player to
/// keep watch-progress flowing while the local `<video>` is paused for
/// casting — the sender still holds the session and the receiver reports
/// its clock here, so we can scrobble with the user's normal cookie auth.
export function subscribeRemotePlayback(
  onChange: (state: RemotePlaybackState) => void,
): () => void {
  const w = window as CastWindow;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const framework = (w.cast as any)?.framework;
  if (!framework) return () => {};
  try {
    const player = new framework.RemotePlayer();
    const controller = new framework.RemotePlayerController(player);
    const emit = () =>
      onChange({
        currentTimeS: player.currentTime ?? 0,
        durationS: player.duration ?? 0,
        isPaused: !!player.isPaused,
        isMediaLoaded: !!player.isMediaLoaded,
      });
    const evt = framework.RemotePlayerEventType;
    const types = [
      evt.CURRENT_TIME_CHANGED,
      evt.DURATION_CHANGED,
      evt.IS_PAUSED_CHANGED,
      evt.IS_MEDIA_LOADED_CHANGED,
      evt.PLAYER_STATE_CHANGED,
    ];
    types.forEach((t) => controller.addEventListener(t, emit));
    emit(); // seed the caller with the current state immediately
    return () => {
      types.forEach((t) => controller.removeEventListener(t, emit));
    };
  } catch (e) {
    devWarn("[cast] RemotePlayer subscription failed", e);
    return () => {};
  }
}

/// End the current cast session, optionally telling the receiver to
/// stop the running media (true) or leave it playing (false — useful
/// if the user just wants to switch back to local).
export function endCastSession(stopReceiver = true): void {
  const w = window as CastWindow;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const framework = (w.cast as any)?.framework;
  if (!framework) return;
  try {
    framework.CastContext.getInstance().endCurrentSession(stopReceiver);
  } catch (e) {
    devWarn("[cast] endCurrentSession failed", e);
  }
}

/// Convenience: build the absolute manifest URL the Cast receiver
/// will fetch. Cast URLs MUST be absolute (relative URLs resolve
/// against the receiver app's origin, not ours), and the receiver
/// can't carry our cookie — so we append the cast token as `?ct=`.
export function buildCastUrl(localPath: string, token: string): string {
  // localPath may already start with `/api/v1/...` (good) or be a
  // full URL (rare; the player only deals in paths). Coerce to a
  // URL relative to the current origin first.
  const absolute = new URL(localPath, window.location.origin);
  absolute.searchParams.set("ct", token);
  return absolute.toString();
}

// ---------------------------------------------------------------------------
// AirPlay (iOS / Safari)
// ---------------------------------------------------------------------------

/// Subscribe to AirPlay-target-availability events on a video element.
/// Returns true while at least one AirPlay-capable device is on the
/// network and discoverable. Safari fires
/// `webkitplaybacktargetavailabilitychanged` whenever the set of
/// reachable devices changes.
export function useAirPlayAvailability(
  videoRef: RefObject<HTMLVideoElement | null>,
): boolean {
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    // The event + the picker method are non-standard WebKit APIs.
    // Type-narrow defensively rather than pulling in WebKit-specific
    // types for one feature.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const v = video as any;
    if (typeof v.webkitShowPlaybackTargetPicker !== "function") {
      return;
    }
    const onChange = (e: Event) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const detail = e as any;
      setAvailable(detail.availability === "available");
    };
    video.addEventListener(
      "webkitplaybacktargetavailabilitychanged",
      onChange as EventListener,
    );
    return () => {
      video.removeEventListener(
        "webkitplaybacktargetavailabilitychanged",
        onChange as EventListener,
      );
    };
  }, [videoRef]);
  return available;
}

/// Open the AirPlay device picker on the given video element. No-op
/// on non-Safari browsers. Safari will route the local <video> to
/// the chosen target; cookies on the original request continue to
/// authenticate segment fetches, so no token gymnastics needed.
export function showAirPlayPicker(video: HTMLVideoElement): void {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const v = video as any;
  if (typeof v.webkitShowPlaybackTargetPicker === "function") {
    try {
      v.webkitShowPlaybackTargetPicker();
    } catch (e) {
      devWarn("[airplay] picker failed", e);
    }
  }
}
