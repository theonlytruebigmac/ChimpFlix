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
    w.__onGCastApiAvailable = (available: boolean) => {
      if (!available) {
        resolve(false);
        return;
      }
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const framework = (w.cast as any)?.framework;
        if (!framework) {
          resolve(false);
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
        resolve(true);
      } catch (e) {
        devError("[cast] init failed", e);
        resolve(false);
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
      if (w.chrome?.cast) resolve(true);
      return;
    }
    const script = document.createElement("script");
    script.src = CAST_SDK_URL;
    script.async = true;
    script.onerror = () => {
      devWarn("[cast] sender SDK failed to load");
      resolve(false);
    };
    document.head.appendChild(script);
  });
  return castSdkLoad;
}

/// Cast session state surfaced to the React tree. `available` flips
/// true when the SDK has loaded AND at least one Cast receiver is
/// discovered on the local network. `connected` is true while a cast
/// session is live (media may not have started yet — see `mediaInfo`).
export interface CastState {
  available: boolean;
  connected: boolean;
  deviceName: string | null;
}

/// Subscribe to Cast SDK state. Returns a CastState that updates as
/// the SDK reports availability + session lifecycle changes.
export function useCastState(): CastState {
  const [state, setState] = useState<CastState>({
    available: false,
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
          // `castState` strings: NO_DEVICES_AVAILABLE / NOT_CONNECTED /
          // CONNECTING / CONNECTED. We treat anything other than the
          // "no devices" state as "user has a cast device they can
          // pick" so the button shows up; the click handler then
          // routes through the SDK picker.
          available: castState !== framework.CastState.NO_DEVICES_AVAILABLE,
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
