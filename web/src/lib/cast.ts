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

/// Default Media Receiver — Google's stock receiver app that can
/// play HLS / DASH / MP4 directly with no custom receiver.
const DEFAULT_RECEIVER_APP_ID = "CC1AD845";

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
    const timeoutMs = 8000;
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
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const framework = (w.cast as any)?.framework;
        if (!framework) {
          finalize(false, "SDK loaded but cast.framework is missing");
          return;
        }
        const context = framework.CastContext.getInstance();
        context.setOptions({
          receiverApplicationId: DEFAULT_RECEIVER_APP_ID,
          // RESUME_SESSION makes the SDK silently re-attach if the
          // user navigates between pages while a cast is active.
          // Without it, the receiver keeps playing but the page loses
          // its handle on the session.
          autoJoinPolicy: framework.AutoJoinPolicy.ORIGIN_SCOPED,
        });
        finalize(true, "ready");
      } catch (e) {
        devError("[cast] init failed", e);
        finalize(false, "init threw");
      }
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
    let removeAvailability: (() => void) | null = null;
    let removeSession: (() => void) | null = null;
    (async () => {
      const loaded = await loadCastSdk();
      if (cancelled || !loaded) return;
      const w = window as CastWindow;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const framework = (w.cast as any)?.framework;
      if (!framework) return;
      const ctx = framework.CastContext.getInstance();

      const sync = () => {
        const session = ctx.getCurrentSession();
        const castState = ctx.getCastState();
        if (cancelled) return;
        setState({
          // SDK loaded successfully — surface the button so the user
          // can discover the affordance even before a receiver wakes
          // up. Picker handles the empty case with "No devices found".
          available: true,
          // `castState` strings: NO_DEVICES_AVAILABLE / NOT_CONNECTED /
          // CONNECTING / CONNECTED. Anything other than the "no devices"
          // state means a receiver is actually reachable right now.
          hasDevices: castState !== framework.CastState.NO_DEVICES_AVAILABLE,
          connected: castState === framework.CastState.CONNECTED,
          deviceName: session?.getCastDevice?.()?.friendlyName ?? null,
        });
      };
      sync();
      const onCastStateChange = () => sync();
      const onSessionStateChange = () => sync();
      ctx.addEventListener(
        framework.CastContextEventType.CAST_STATE_CHANGED,
        onCastStateChange,
      );
      ctx.addEventListener(
        framework.CastContextEventType.SESSION_STATE_CHANGED,
        onSessionStateChange,
      );
      removeAvailability = () =>
        ctx.removeEventListener(
          framework.CastContextEventType.CAST_STATE_CHANGED,
          onCastStateChange,
        );
      removeSession = () =>
        ctx.removeEventListener(
          framework.CastContextEventType.SESSION_STATE_CHANGED,
          onSessionStateChange,
        );
    })();
    return () => {
      cancelled = true;
      removeAvailability?.();
      removeSession?.();
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

/// Open the Cast picker. If the user selects a device, load the given
/// media on it. Returns true if the cast session started, false on
/// user cancel or SDK error.
export async function startCastSession(media: CastMediaPayload): Promise<boolean> {
  const w = window as CastWindow;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const chromeCast = (w.chrome as any)?.cast;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const framework = (w.cast as any)?.framework;
  if (!chromeCast || !framework) {
    devWarn("[cast] SDK not loaded; cannot start session");
    return false;
  }
  try {
    const ctx = framework.CastContext.getInstance();
    let session = ctx.getCurrentSession();
    if (!session) {
      // requestSession resolves with a "success"/"cancel"/etc. string
      // OR throws on hard failure. Either way, treat anything that
      // isn't a live session as "user backed out."
      await ctx.requestSession();
      session = ctx.getCurrentSession();
    }
    if (!session) return false;
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
    devWarn("[cast] session start failed", e);
    return false;
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
