"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
} from "react";
import type Hls from "hls.js";
import { plexImage } from "@/lib/image";
import { getPrefs, updatePrefs, usePrefs } from "@/lib/prefs";
import type { Marker, MediaStream } from "@/lib/plex-types";

function streamLabel(s: MediaStream): string {
  if (s.displayTitle && s.displayTitle.trim()) return s.displayTitle;
  if (s.language && s.language.trim()) return s.language;
  if (s.languageCode && s.languageCode.trim()) {
    return s.languageCode.toUpperCase();
  }
  return `Track ${s.id}`;
}

// crypto.randomUUID() requires a secure context (HTTPS or localhost). Over
// plain HTTP on a LAN IP it's undefined, so we fall back to a non-crypto
// random ID — Plex only uses this as an opaque session identifier, not for
// security.
function generateSessionId(): string {
  if (
    typeof crypto !== "undefined" &&
    typeof crypto.randomUUID === "function"
  ) {
    return crypto.randomUUID();
  }
  const rand = () => Math.random().toString(36).slice(2, 10);
  return `${Date.now().toString(36)}-${rand()}-${rand()}`;
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${m}:${String(s).padStart(2, "0")}`;
}

type EpisodeSibling = {
  ratingKey: string;
  title: string;
  thumb?: string;
  summary?: string;
  duration?: number;
  viewOffset?: number;
  index?: number;
  parentTitle?: string;
};

type PlayerProps = {
  ratingKey: string;
  title: string;
  subtitle?: string;
  duration?: number;
  viewOffset?: number;
  backToRatingKey?: string;
  nextRatingKey?: string;
  nextLabel?: string;
  nextThumb?: string;
  markers?: Marker[];
  audioStreams?: MediaStream[];
  subtitleStreams?: MediaStream[];
  // Plex Part ID (Media[0].Part[0].id). Required for the /api/subtitle
  // endpoint to hit Plex's /library/parts/<partId> direct-fetch URL.
  partId?: number;
  seasonEpisodes?: EpisodeSibling[];
};

// Pick the stream ID that matches the user's preferred language code, if
// any. Returns null when the prefs are unset or no stream matches — in that
// case the transcoder's default selection applies.
function streamMatchingLanguage(
  streams: MediaStream[] | undefined,
  langCode: string,
): number | null {
  if (!streams || !langCode) return null;
  const target = langCode.toLowerCase();
  const hit = streams.find((s) => s.languageCode?.toLowerCase() === target);
  return hit ? hit.id : null;
}

const COUNTDOWN_WINDOW_SECONDS = 10;

function activeMarker(currentMs: number, markers?: Marker[]): Marker | null {
  if (!markers) return null;
  for (const m of markers) {
    if (currentMs >= m.startMs && currentMs <= m.endMs) return m;
  }
  return null;
}

function markerLabel(m: Marker): string {
  if (m.type === "credits") return "Skip Credits";
  if (m.type === "commercial") return "Skip Ad";
  return "Skip Intro";
}

export function Player({
  ratingKey,
  title,
  subtitle,
  duration,
  viewOffset,
  backToRatingKey,
  nextRatingKey,
  nextLabel,
  nextThumb,
  markers,
  audioStreams,
  subtitleStreams,
  partId,
  seasonEpisodes,
}: PlayerProps) {
  const router = useRouter();
  const [prefs] = usePrefs();
  const backHref = backToRatingKey ? `/?title=${backToRatingKey}` : "/";
  const nextHref = nextRatingKey ? `/watch/${nextRatingKey}` : null;
  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hideTimerRef = useRef<number | null>(null);
  const hlsRef = useRef<Hls | null>(null);

  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [videoDuration, setVideoDuration] = useState(0);
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showControls, setShowControls] = useState(true);
  const [autoplayBlocked, setAutoplayBlocked] = useState(false);
  // Selected Plex stream IDs. `null` means "let the transcoder pick its
  // default" (we omit the param from the URL entirely). For subtitles, -1
  // means "explicitly off" — we send subtitleStreamID=0 to Plex which is the
  // server's signal to disable subtitle rendering.
  const [audioStreamId, setAudioStreamId] = useState<number | null>(() =>
    streamMatchingLanguage(audioStreams, getPrefs().audioLanguage),
  );
  const [subtitleStreamId, setSubtitleStreamId] = useState<number | null>(() => {
    const pref = getPrefs().subtitleLanguage;
    if (pref === "off") return -1;
    return streamMatchingLanguage(subtitleStreams, pref);
  });
  const [tracksOpen, setTracksOpen] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [speedOpen, setSpeedOpen] = useState(false);
  const [autoNextCancelled, setAutoNextCancelled] = useState(false);
  const [episodesOpen, setEpisodesOpen] = useState(false);
  const [pipActive, setPipActive] = useState(false);
  const [showRemaining, setShowRemaining] = useState(true);

  // Wraps video.play() so we can detect Firefox/Chrome autoplay rejection
  // and prompt the viewer with a click-to-play overlay.
  const attemptPlay = useCallback(async () => {
    const v = videoRef.current;
    if (!v) return;
    try {
      await v.play();
      setAutoplayBlocked(false);
    } catch (err) {
      if (err instanceof DOMException && err.name === "NotAllowedError") {
        setAutoplayBlocked(true);
      }
    }
  }, []);

  // ── HLS setup ─────────────────────────────────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const session = generateSessionId();
    const params = new URLSearchParams({
      path: `/library/metadata/${ratingKey}`,
      protocol: "hls",
      mediaIndex: "0",
      partIndex: "0",
      fastSeek: "1",
      copyts: "1",
      videoQuality: "100",
      maxVideoBitrate: "20000",
      videoResolution: "1920x1080",
      audioBoost: "100",
      subtitleSize: "100",
      directStream: "1",
      directPlay: "0",
      hasMDE: "1",
      session,
    });
    if (audioStreamId !== null) {
      params.set("audioStreamID", String(audioStreamId));
    }
    // Subtitle is intentionally NOT sent to the transcoder. Some Plex servers
    // silently drop subtitles from the HLS manifest regardless of the params
    // we send. Instead we fetch the subtitle file separately and render it
    // via a <track> element on the <video>, which is independent of the
    // transcode pipeline and works on every Plex server.
    const url = `/api/plex/video/:/transcode/universal/start.m3u8?${params}`;

    // When changing audio/subtitle mid-playback we want to land back at
    // roughly the same spot. Prefer the live currentTime over the initial
    // viewOffset so a user who paused at 0:42:10 and switched subs doesn't
    // get yanked back to where they started the session.
    const liveTime = video.currentTime;
    const resumeMs =
      liveTime > 1 ? Math.floor(liveTime * 1000) : viewOffset ?? 0;

    function applyResume() {
      if (!video) return;
      if (resumeMs > 1000) {
        video.currentTime = resumeMs / 1000;
      }
    }

    let cancelled = false;
    let cleanup: () => void = () => {};

    (async () => {
      const HlsModule = (await import("hls.js")).default;
      if (cancelled) return;

      if (HlsModule.isSupported()) {
        const hls = new HlsModule({
          // Run demuxing on a worker thread so segment parsing doesn't
          // contend with React renders / GC on the main thread.
          enableWorker: true,
          // Smaller buffer targets keep the time-to-first-frame down.
          // Default forward buffer is 30s/600s — plenty for VOD; we drop
          // back-buffer to 30s to keep memory use modest on long sessions.
          backBufferLength: 30,
          // Faster failure detection on cold transcoder starts. Plex's
          // first manifest can take 1-3s; 6s timeout is enough on a LAN
          // and keeps spinners from feeling like hangs.
          manifestLoadingTimeOut: 6000,
          manifestLoadingMaxRetry: 2,
          levelLoadingTimeOut: 6000,
          // Segment fetches still get a generous timeout since the
          // transcoder may be encoding mid-flight.
          fragLoadingTimeOut: 20000,
          // Start ABR with an optimistic estimate so the first level is
          // chosen quickly. Default is 5e5 (500kbps) which under-shoots
          // a LAN connection.
          abrEwmaDefaultEstimate: 5_000_000,
        });
        hlsRef.current = hls;
        hls.loadSource(url);
        hls.attachMedia(video);
        hls.on(HlsModule.Events.MANIFEST_PARSED, () => {
          applyResume();
          attemptPlay();
        });
        hls.on(HlsModule.Events.ERROR, (_event, data) => {
          if (data.fatal) {
            setError(`${data.type} / ${data.details}`);
          }
        });
        cleanup = () => {
          hlsRef.current = null;
          hls.destroy();
        };
      } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
        video.src = url;
        const onLoaded = () => {
          applyResume();
          attemptPlay();
        };
        video.addEventListener("loadedmetadata", onLoaded, { once: true });
        cleanup = () =>
          video.removeEventListener("loadedmetadata", onLoaded);
      } else {
        setError("HLS playback isn't supported in this browser.");
      }
    })().catch((e) => {
      if (cancelled) return;
      setError(e instanceof Error ? e.message : String(e));
    });

    return () => {
      cancelled = true;
      cleanup();
    };
    // Re-running this effect on audio change tears down hls and requests a
    // new manifest with the chosen audioStreamID. Subtitle changes don't
    // belong here — those are rendered via a separate <track> element and
    // swap instantly without touching the HLS stream.
  }, [ratingKey, viewOffset, attemptPlay, audioStreamId]);

  // ── Video state subscriptions ─────────────────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const onLoadedMetadata = () => setVideoDuration(video.duration);
    const onTimeUpdate = () => setCurrentTime(video.currentTime);
    const onPlay = () => {
      setPlaying(true);
      setAutoplayBlocked(false);
    };
    const onPause = () => setPlaying(false);
    const onWaiting = () => setLoading(true);
    const onCanPlay = () => {
      setLoading(false);
      // Try once more in case MANIFEST_PARSED fired before user activation
      // had reached this document.
      if (video.paused && !autoplayBlocked) {
        attemptPlay();
      }
    };
    const onPlaying = () => {
      setLoading(false);
      setPlaying(true);
      setAutoplayBlocked(false);
    };
    const onVolumeChange = () => {
      setMuted(video.muted);
      setVolume(video.volume);
    };

    video.addEventListener("loadedmetadata", onLoadedMetadata);
    video.addEventListener("timeupdate", onTimeUpdate);
    video.addEventListener("play", onPlay);
    video.addEventListener("pause", onPause);
    video.addEventListener("waiting", onWaiting);
    video.addEventListener("canplay", onCanPlay);
    video.addEventListener("playing", onPlaying);
    video.addEventListener("volumechange", onVolumeChange);

    return () => {
      video.removeEventListener("loadedmetadata", onLoadedMetadata);
      video.removeEventListener("timeupdate", onTimeUpdate);
      video.removeEventListener("play", onPlay);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("waiting", onWaiting);
      video.removeEventListener("canplay", onCanPlay);
      video.removeEventListener("playing", onPlaying);
      video.removeEventListener("volumechange", onVolumeChange);
    };
  }, [attemptPlay, autoplayBlocked]);

  // ── Apply persisted preferences on mount ──────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const prefs = getPrefs();
    video.volume = prefs.volume;
    video.muted = prefs.muted;
    video.playbackRate = prefs.playbackRate;
    setPlaybackRate(prefs.playbackRate);
  }, []);

  // ── Force our <track> subtitle to display ────────────────────────────────
  // hls.js may inject its own text tracks (often empty for HLS without
  // sidecar VTT) which compete with our React-managed <track>. The `default`
  // attribute on <track> only wins when no other track is set, so we
  // explicitly disable everything else and force our chosen one to "showing"
  // whenever the subtitle selection changes.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    // The text track our <track> element added is the last one in the list
    // (React appended it after hls.js's tracks). We walk the whole list and
    // disable every track that isn't ours, then enable ours.
    function applyTrackMode() {
      if (!video) return;
      const tracks = video.textTracks;
      const wantsSubs = subtitleStreamId !== null && subtitleStreamId !== -1;
      for (let i = 0; i < tracks.length; i++) {
        const t = tracks[i];
        // Match by label since React's <track label> propagates to the
        // resulting TextTrack — the only reliable identifier across
        // hls.js-injected vs React-injected tracks.
        const isOurs =
          wantsSubs &&
          t.label ===
            (subtitleStreams?.find((s) => s.id === subtitleStreamId)
              ? streamLabel(
                  subtitleStreams.find((s) => s.id === subtitleStreamId)!,
                )
              : "Subtitles");
        t.mode = isOurs ? "showing" : "disabled";
      }
    }
    applyTrackMode();
    // hls.js / the <track> load happens asynchronously after the initial
    // render. Watch for additions and reapply.
    const tracks = video.textTracks;
    tracks.addEventListener("addtrack", applyTrackMode);
    tracks.addEventListener("change", applyTrackMode);
    return () => {
      tracks.removeEventListener("addtrack", applyTrackMode);
      tracks.removeEventListener("change", applyTrackMode);
    };
  }, [subtitleStreamId, subtitleStreams]);

  // ── Fullscreen tracking ───────────────────────────────────────────────────
  useEffect(() => {
    const onChange = () =>
      setIsFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  // ── Picture-in-picture tracking ───────────────────────────────────────────
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    const onEnter = () => setPipActive(true);
    const onLeave = () => setPipActive(false);
    v.addEventListener("enterpictureinpicture", onEnter);
    v.addEventListener("leavepictureinpicture", onLeave);
    return () => {
      v.removeEventListener("enterpictureinpicture", onEnter);
      v.removeEventListener("leavepictureinpicture", onLeave);
    };
  }, []);

  // ── Timeline reporting (Plex Continue Watching) ───────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    function report(state: "playing" | "paused" | "stopped") {
      if (!video) return;
      const time = Math.floor(video.currentTime * 1000);
      const dur = Math.floor((video.duration || (duration ?? 0) / 1000) * 1000);
      if (!Number.isFinite(time) || !Number.isFinite(dur) || dur <= 0) return;
      const params = new URLSearchParams({
        ratingKey,
        key: `/library/metadata/${ratingKey}`,
        state,
        time: String(time),
        duration: String(dur),
        hasMDE: "1",
      });
      fetch(`/api/plex/:/timeline?${params}`, { keepalive: true }).catch(
        () => {},
      );
    }

    const interval = window.setInterval(() => {
      if (!video.paused && !video.ended) report("playing");
    }, 10000);

    const onPause = () => report("paused");
    const onPlay = () => report("playing");
    const onEnded = () => report("stopped");
    video.addEventListener("pause", onPause);
    video.addEventListener("play", onPlay);
    video.addEventListener("ended", onEnded);

    return () => {
      window.clearInterval(interval);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("play", onPlay);
      video.removeEventListener("ended", onEnded);
      report("stopped");
    };
  }, [ratingKey, duration]);

  // ── Auto-advance to next episode ──────────────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !nextHref) return;
    function onEnded() {
      if (!autoNextCancelled && prefs.autoplayNext && nextHref) {
        router.push(nextHref);
      }
    }
    video.addEventListener("ended", onEnded);
    return () => video.removeEventListener("ended", onEnded);
  }, [nextHref, router, autoNextCancelled, prefs.autoplayNext]);

  // ── Idle-hide controls ────────────────────────────────────────────────────
  const resetHide = useCallback(() => {
    setShowControls(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      const v = videoRef.current;
      if (v && !v.paused) setShowControls(false);
    }, 3000);
  }, []);

  // ── Imperative controls ───────────────────────────────────────────────────
  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) v.play().catch(() => {});
    else v.pause();
  }, []);

  const seekBy = useCallback((delta: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.currentTime = Math.max(0, Math.min(v.duration || 0, v.currentTime + delta));
  }, []);

  const seekTo = useCallback((time: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.currentTime = Math.max(0, Math.min(v.duration || 0, time));
  }, []);

  const toggleMute = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
    updatePrefs({ muted: v.muted });
  }, []);

  const toggleFullscreen = useCallback(() => {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      containerRef.current?.requestFullscreen().catch(() => {});
    }
  }, []);

  const togglePip = useCallback(async () => {
    const v = videoRef.current;
    if (!v) return;
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
      } else if (typeof v.requestPictureInPicture === "function") {
        await v.requestPictureInPicture();
      }
    } catch {
      // PiP can be blocked by browser policy or unsupported on the source —
      // best-effort, no fatal error needed.
    }
  }, []);

  // Switching audio/subtitle streams requires a new transcode manifest from
  // Plex — there's no in-stream switch when subtitles are burned into the
  // video. The HLS effect's dependency on `audioStreamId`/`subtitleStreamId`
  // does the heavy lifting; here we just persist the choice and update state.
  const selectAudioStream = useCallback(
    (id: number) => {
      const match = audioStreams?.find((s) => s.id === id);
      updatePrefs({ audioLanguage: match?.languageCode ?? "" });
      setAudioStreamId(id);
    },
    [audioStreams],
  );

  const selectSubtitleStream = useCallback(
    (id: number) => {
      if (id === -1) {
        updatePrefs({ subtitleLanguage: "off" });
      } else {
        const match = subtitleStreams?.find((s) => s.id === id);
        updatePrefs({ subtitleLanguage: match?.languageCode ?? "" });
      }
      setSubtitleStreamId(id);
    },
    [subtitleStreams],
  );

  const toggleSubtitles = useCallback(() => {
    if (subtitleStreamId === -1 || subtitleStreamId === null) {
      // Re-enable: prefer the language the user previously chose, fall back
      // to the first available stream.
      const pref = getPrefs().subtitleLanguage;
      const target =
        streamMatchingLanguage(subtitleStreams, pref) ??
        subtitleStreams?.[0]?.id;
      if (target !== undefined) selectSubtitleStream(target);
    } else {
      selectSubtitleStream(-1);
    }
  }, [subtitleStreamId, subtitleStreams, selectSubtitleStream]);

  const setVolumeValue = useCallback((value: number) => {
    const v = videoRef.current;
    if (!v) return;
    const clamped = Math.max(0, Math.min(1, value));
    v.volume = clamped;
    const wasMuted = v.muted;
    if (clamped > 0 && wasMuted) v.muted = false;
    updatePrefs({
      volume: clamped,
      muted: clamped === 0 ? wasMuted : false,
    });
  }, []);

  const setSpeed = useCallback((rate: number) => {
    const v = videoRef.current;
    if (!v) return;
    v.playbackRate = rate;
    setPlaybackRate(rate);
    updatePrefs({ playbackRate: rate });
  }, []);

  // ── Keyboard shortcuts ────────────────────────────────────────────────────
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      switch (e.key) {
        case " ":
        case "k":
        case "K":
          e.preventDefault();
          togglePlay();
          resetHide();
          break;
        case "ArrowLeft":
        case "j":
        case "J":
          seekBy(-10);
          resetHide();
          break;
        case "ArrowRight":
        case "l":
        case "L":
          seekBy(10);
          resetHide();
          break;
        case "ArrowUp": {
          e.preventDefault();
          const v = videoRef.current;
          if (v) setVolumeValue(Math.min(1, (v.muted ? 0 : v.volume) + 0.05));
          resetHide();
          break;
        }
        case "ArrowDown": {
          e.preventDefault();
          const v = videoRef.current;
          if (v) setVolumeValue(Math.max(0, (v.muted ? 0 : v.volume) - 0.05));
          resetHide();
          break;
        }
        case "Home": {
          e.preventDefault();
          seekTo(0);
          resetHide();
          break;
        }
        case "End": {
          e.preventDefault();
          const v = videoRef.current;
          if (v && v.duration) seekTo(v.duration - 1);
          resetHide();
          break;
        }
        case "f":
        case "F":
          toggleFullscreen();
          resetHide();
          break;
        case "m":
        case "M":
          toggleMute();
          resetHide();
          break;
        case "c":
        case "C":
          toggleSubtitles();
          resetHide();
          break;
        case "p":
        case "P":
          togglePip();
          resetHide();
          break;
        case ">":
        case ".":
          // Shift+. → ">". Bump speed up to next preset.
          if (e.shiftKey || e.key === ">") {
            const cur = videoRef.current?.playbackRate ?? 1;
            const next = SPEED_OPTIONS.find((o) => o > cur);
            if (next !== undefined) setSpeed(next);
            resetHide();
          }
          break;
        case "<":
        case ",":
          if (e.shiftKey || e.key === "<") {
            const cur = videoRef.current?.playbackRate ?? 1;
            const prev = [...SPEED_OPTIONS].reverse().find((o) => o < cur);
            if (prev !== undefined) setSpeed(prev);
            resetHide();
          }
          break;
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    togglePlay,
    seekBy,
    seekTo,
    toggleFullscreen,
    toggleMute,
    toggleSubtitles,
    togglePip,
    setVolumeValue,
    setSpeed,
    resetHide,
  ]);

  return (
    <div
      ref={containerRef}
      onMouseMove={resetHide}
      onPointerDown={resetHide}
      className={`fixed inset-0 z-50 select-none bg-black ${
        showControls ? "" : "cursor-none"
      }`}
    >
      <video
        ref={videoRef}
        playsInline
        autoPlay
        onClick={togglePlay}
        crossOrigin="anonymous"
        className="h-full w-full bg-black"
      >
        {subtitleStreamId !== null && subtitleStreamId !== -1 && (
          <track
            // `key` forces a fresh <track> element when the language changes
            // so the browser drops the previous cues and fetches the new src.
            key={subtitleStreamId}
            kind="subtitles"
            src={`/api/subtitle/${subtitleStreamId}?ratingKey=${ratingKey}`}
            srcLang={
              subtitleStreams?.find((s) => s.id === subtitleStreamId)
                ?.languageCode ?? "und"
            }
            label={
              subtitleStreams?.find((s) => s.id === subtitleStreamId)
                ? streamLabel(
                    subtitleStreams.find((s) => s.id === subtitleStreamId)!,
                  )
                : "Subtitles"
            }
            default
          />
        )}
      </video>

      {error && <ErrorOverlay message={error} />}
      {loading && !error && !autoplayBlocked && <LoadingSpinner />}
      {autoplayBlocked && !error && (
        <BigPlayButton onClick={attemptPlay} />
      )}
      {(() => {
        const m = activeMarker(currentTime * 1000, markers);
        if (!m) return null;
        return (
          <button
            type="button"
            onClick={() => seekTo(m.endMs / 1000)}
            className="pointer-events-auto absolute bottom-32 right-8 z-30 rounded-md border border-white/30 bg-white/95 px-6 py-2.5 text-sm font-semibold text-black shadow-2xl transition-all hover:scale-[1.03] hover:bg-white"
          >
            {markerLabel(m)}
          </button>
        );
      })()}
      {nextHref &&
        nextLabel &&
        !autoNextCancelled &&
        prefs.autoplayNext &&
        videoDuration > 0 &&
        videoDuration - currentTime <= COUNTDOWN_WINDOW_SECONDS &&
        videoDuration - currentTime > 0 && (
          <NextEpisodeCountdown
            secondsLeft={Math.max(0, Math.ceil(videoDuration - currentTime))}
            href={nextHref}
            label={nextLabel}
            thumb={nextThumb}
            onCancel={() => setAutoNextCancelled(true)}
          />
        )}

      <div
        className={`pointer-events-none absolute inset-0 transition-opacity duration-200 ${
          showControls ? "opacity-100" : "opacity-0"
        }`}
      >
        {/* Top bar — Netflix keeps this minimal: just a back affordance. */}
        <div className="pointer-events-auto absolute inset-x-0 top-0 bg-linear-to-b from-black/80 to-transparent">
          <div className="flex items-start gap-6 px-8 py-5">
            <Link
              href={backHref}
              aria-label="Back"
              className="flex items-center gap-2 rounded-full p-2 -m-2 text-white/85 transition-colors hover:text-white"
            >
              <BackIcon />
              <span className="text-sm font-medium">Back</span>
            </Link>
          </div>
        </div>

        {/* Bottom controls. Netflix layout:
              [progress bar          ] [time remaining]
              [play|10|10|vol]  [TITLE]  [next|episodes|tracks|speed|pip|fs] */}
        <div className="pointer-events-auto absolute inset-x-0 bottom-0 bg-linear-to-t from-black/85 to-transparent px-8 pb-6 pt-16">
          <div className="flex items-center gap-3">
            <div className="grow">
              <ProgressBar
                currentTime={currentTime}
                duration={videoDuration}
                onSeek={seekTo}
              />
            </div>
            <button
              type="button"
              onClick={() => setShowRemaining((s) => !s)}
              aria-label="Toggle time remaining"
              className="shrink-0 text-sm tabular-nums text-white/85 transition-colors hover:text-white"
            >
              {showRemaining
                ? `-${formatTime(Math.max(0, videoDuration - currentTime))}`
                : formatTime(currentTime)}
            </button>
          </div>

          <div className="mt-2 flex items-center gap-4">
            <div className="flex shrink-0 items-center gap-5">
              <IconButton
                onClick={togglePlay}
                aria-label={playing ? "Pause" : "Play"}
              >
                {playing ? <PauseIcon /> : <PlayIcon />}
              </IconButton>
              <IconButton
                onClick={() => seekBy(-10)}
                aria-label="Skip back 10 seconds"
              >
                <Rewind10Icon />
              </IconButton>
              <IconButton
                onClick={() => seekBy(10)}
                aria-label="Skip forward 10 seconds"
              >
                <Forward10Icon />
              </IconButton>
              <VolumeControl
                muted={muted}
                volume={volume}
                onToggleMute={toggleMute}
                onVolumeChange={setVolumeValue}
              />
            </div>
            <div className="min-w-0 grow text-center">
              <div className="truncate text-sm font-semibold leading-tight">
                {title}
              </div>
              {subtitle && (
                <div className="mt-0.5 truncate text-xs text-white/70">
                  {subtitle}
                </div>
              )}
            </div>
            <div className="flex shrink-0 items-center gap-5">
              {nextHref && (
                <Link
                  href={nextHref}
                  aria-label={nextLabel ?? "Next episode"}
                  title={nextLabel ?? "Next episode"}
                  className="flex h-10 items-center justify-center text-white/90 transition-colors hover:text-white"
                >
                  <NextEpisodeIcon />
                </Link>
              )}
              {seasonEpisodes && seasonEpisodes.length > 1 && (
                <EpisodesControl
                  open={episodesOpen}
                  episodes={seasonEpisodes}
                  currentRatingKey={ratingKey}
                  onToggle={() => setEpisodesOpen((o) => !o)}
                  onClose={() => setEpisodesOpen(false)}
                />
              )}
              {((audioStreams && audioStreams.length > 1) ||
                (subtitleStreams && subtitleStreams.length > 0)) && (
                <TracksControl
                  audioStreams={audioStreams ?? []}
                  subtitleStreams={subtitleStreams ?? []}
                  currentAudioId={audioStreamId}
                  currentSubtitleId={subtitleStreamId}
                  open={tracksOpen}
                  onToggle={() => setTracksOpen((o) => !o)}
                  onClose={() => setTracksOpen(false)}
                  onAudioSelect={selectAudioStream}
                  onSubtitleSelect={selectSubtitleStream}
                />
              )}
              <SpeedControl
                rate={playbackRate}
                open={speedOpen}
                onToggle={() => setSpeedOpen((o) => !o)}
                onClose={() => setSpeedOpen(false)}
                onSelect={setSpeed}
              />
              <IconButton
                onClick={togglePip}
                aria-label={pipActive ? "Exit picture-in-picture" : "Picture-in-picture"}
                aria-pressed={pipActive}
              >
                <PipIcon />
              </IconButton>
              <IconButton
                onClick={toggleFullscreen}
                aria-label={isFullscreen ? "Exit fullscreen" : "Fullscreen"}
              >
                {isFullscreen ? <FullscreenExitIcon /> : <FullscreenIcon />}
              </IconButton>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── UI subcomponents ────────────────────────────────────────────────────────

function IconButton({
  children,
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      {...props}
      className={`flex h-10 w-10 items-center justify-center text-white/90 transition-colors hover:text-white ${className}`}
    >
      {children}
    </button>
  );
}

function BigPlayButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label="Play"
      className="absolute inset-0 z-20 flex cursor-pointer items-center justify-center bg-black/30 transition-colors hover:bg-black/50"
    >
      <div className="flex h-24 w-24 items-center justify-center rounded-full bg-white text-black shadow-2xl transition-transform hover:scale-105">
        <svg
          width="44"
          height="44"
          viewBox="0 0 24 24"
          fill="currentColor"
          aria-hidden
        >
          <path d="M7 4l13 8-13 8V4z" />
        </svg>
      </div>
    </button>
  );
}

function TracksControl({
  audioStreams,
  subtitleStreams,
  currentAudioId,
  currentSubtitleId,
  open,
  onToggle,
  onClose,
  onAudioSelect,
  onSubtitleSelect,
}: {
  audioStreams: MediaStream[];
  subtitleStreams: MediaStream[];
  // null = transcoder default (no explicit ID sent). For subs, -1 = off.
  currentAudioId: number | null;
  currentSubtitleId: number | null;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onAudioSelect: (id: number) => void;
  onSubtitleSelect: (id: number) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) onClose();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open, onClose]);

  // The "currently active" highlight has to fall back to the Plex-marked
  // `selected` stream when the user hasn't explicitly chosen one — that's
  // what the transcoder is actually playing.
  const effectiveAudioId =
    currentAudioId ?? audioStreams.find((s) => s.selected)?.id ?? null;
  const effectiveSubtitleId =
    currentSubtitleId ??
    subtitleStreams.find((s) => s.selected)?.id ??
    null;

  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={onToggle}
        aria-label="Audio and subtitles"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <CaptionsIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          className="absolute bottom-full right-0 mb-3 grid w-md grid-cols-2 overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm"
        >
          <StreamColumn
            label="Audio"
            streams={audioStreams}
            currentId={effectiveAudioId}
            onSelect={onAudioSelect}
            offOption={false}
          />
          <StreamColumn
            label="Subtitles"
            streams={subtitleStreams}
            currentId={effectiveSubtitleId}
            onSelect={onSubtitleSelect}
            offOption={true}
          />
        </div>
      )}
    </div>
  );
}

function StreamColumn({
  label,
  streams,
  currentId,
  onSelect,
  offOption,
}: {
  label: string;
  streams: MediaStream[];
  currentId: number | null;
  onSelect: (id: number) => void;
  offOption: boolean;
}) {
  return (
    <div className="border-r border-white/10 last:border-r-0">
      <div className="border-b border-white/10 px-4 py-3 text-[0.7rem] font-semibold uppercase tracking-wider text-white/60">
        {label}
      </div>
      <ul className="max-h-72 overflow-y-auto py-2">
        {offOption && (
          <TrackRow
            label="Off"
            active={currentId === -1}
            onClick={() => onSelect(-1)}
          />
        )}
        {streams.map((s) => (
          <TrackRow
            key={s.id}
            label={streamLabel(s)}
            active={currentId === s.id}
            onClick={() => onSelect(s.id)}
          />
        ))}
        {streams.length === 0 && !offOption && (
          <li className="px-4 py-2 text-sm text-white/50">None available</li>
        )}
      </ul>
    </div>
  );
}

function TrackRow({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        role="menuitemradio"
        aria-checked={active}
        className={`flex w-full items-center gap-2 px-4 py-2 text-left text-sm transition-colors ${
          active ? "text-white" : "text-white/75 hover:text-white"
        }`}
      >
        <span
          aria-hidden
          className={`flex h-4 w-4 shrink-0 items-center justify-center text-(--color-accent) ${
            active ? "opacity-100" : "opacity-0"
          }`}
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="3"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <polyline points="20 6 9 17 4 12" />
          </svg>
        </span>
        <span className="truncate">{label}</span>
      </button>
    </li>
  );
}

function NextEpisodeCountdown({
  secondsLeft,
  href,
  label,
  thumb,
  onCancel,
}: {
  secondsLeft: number;
  href: string;
  label: string;
  thumb?: string;
  onCancel: () => void;
}) {
  const img = thumb ? plexImage(thumb, 480, 270) : null;
  return (
    <div className="pointer-events-auto absolute bottom-28 right-8 z-30 w-80 overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm">
      {img && (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={img}
          alt=""
          className="aspect-video w-full object-cover"
        />
      )}
      <div className="px-4 pb-4 pt-3">
        <div className="text-xs uppercase tracking-wider text-white/60">
          Next episode in {secondsLeft}s
        </div>
        <div className="mt-1 line-clamp-2 text-base font-semibold leading-tight">
          {label}
        </div>
        <div className="mt-3 flex gap-2">
          <Link
            href={href}
            className="flex-1 rounded bg-white px-3 py-1.5 text-center text-sm font-semibold text-black transition-colors hover:bg-white/85"
          >
            Watch now
          </Link>
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-white/40 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:border-white"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

function VolumeControl({
  muted,
  volume,
  onToggleMute,
  onVolumeChange,
}: {
  muted: boolean;
  volume: number;
  onToggleMute: () => void;
  onVolumeChange: (v: number) => void;
}) {
  const [hovered, setHovered] = useState(false);
  const trackRef = useRef<HTMLDivElement>(null);
  const effectiveVolume = muted ? 0 : volume;

  function pointToVolume(clientX: number): number {
    const track = trackRef.current;
    if (!track) return effectiveVolume;
    const rect = track.getBoundingClientRect();
    return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
  }

  function onPointerDown(e: React.PointerEvent<HTMLDivElement>) {
    e.preventDefault();
    onVolumeChange(pointToVolume(e.clientX));
    const onMove = (ev: PointerEvent) => onVolumeChange(pointToVolume(ev.clientX));
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  return (
    <div
      className="flex items-center"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <IconButton
        onClick={onToggleMute}
        aria-label={muted ? "Unmute" : "Mute"}
      >
        {muted || volume === 0 ? <VolumeMutedIcon /> : <VolumeIcon />}
      </IconButton>
      <div
        ref={trackRef}
        onPointerDown={onPointerDown}
        role="slider"
        aria-label="Volume"
        aria-valuemin={0}
        aria-valuemax={1}
        aria-valuenow={effectiveVolume}
        className={`relative h-1 cursor-pointer overflow-hidden rounded-full bg-white/30 transition-all duration-150 ${
          hovered ? "ml-1 w-24 opacity-100" : "ml-0 w-0 opacity-0"
        }`}
      >
        <div
          className="absolute inset-y-0 left-0 bg-white"
          style={{ width: `${effectiveVolume * 100}%` }}
        />
      </div>
    </div>
  );
}

const SPEED_OPTIONS = [0.5, 0.75, 1, 1.25, 1.5, 2] as const;

function SpeedControl({
  rate,
  open,
  onToggle,
  onClose,
  onSelect,
}: {
  rate: number;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onSelect: (rate: number) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) onClose();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open, onClose]);

  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={onToggle}
        aria-label="Playback speed"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <SpeedIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          className="absolute bottom-full right-0 mb-3 w-32 overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm"
        >
          <ul className="py-2">
            {SPEED_OPTIONS.map((opt) => {
              const active = Math.abs(opt - rate) < 0.001;
              return (
                <li key={opt}>
                  <button
                    type="button"
                    onClick={() => {
                      onSelect(opt);
                      onClose();
                    }}
                    role="menuitemradio"
                    aria-checked={active}
                    className={`flex w-full items-center gap-2 px-4 py-1.5 text-left text-sm transition-colors ${
                      active ? "text-white" : "text-white/75 hover:text-white"
                    }`}
                  >
                    <span
                      aria-hidden
                      className={`flex h-4 w-4 shrink-0 items-center justify-center text-(--color-accent) ${
                        active ? "opacity-100" : "opacity-0"
                      }`}
                    >
                      <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="3"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      >
                        <polyline points="20 6 9 17 4 12" />
                      </svg>
                    </span>
                    <span className="tabular-nums">
                      {opt === 1 ? "Normal" : `${opt}×`}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

function SpeedIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="12" cy="13" r="8" />
      <polyline points="12 9 12 13 15 15" />
      <line x1="9" y1="3" x2="15" y2="3" />
    </svg>
  );
}

function NextEpisodeIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M5 4l11 8-11 8V4z" />
      <rect x="17" y="4" width="2.5" height="16" rx="0.5" />
    </svg>
  );
}

function EpisodesControl({
  open,
  episodes,
  currentRatingKey,
  onToggle,
  onClose,
}: {
  open: boolean;
  episodes: EpisodeSibling[];
  currentRatingKey: string;
  onToggle: () => void;
  onClose: () => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (!wrapRef.current?.contains(e.target as Node)) onClose();
    }
    function onEsc(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onEsc);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onEsc);
    };
  }, [open, onClose]);

  const seasonLabel = episodes[0]?.parentTitle;

  return (
    <div ref={wrapRef} className="relative">
      <IconButton
        onClick={onToggle}
        aria-label="Episodes"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <EpisodesIcon />
      </IconButton>
      {open && (
        <div
          role="menu"
          className="absolute bottom-full right-0 mb-3 w-md overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm"
        >
          {seasonLabel && (
            <div className="border-b border-white/10 px-4 py-3 text-sm font-semibold">
              {seasonLabel}
            </div>
          )}
          <ul className="max-h-112 overflow-y-auto">
            {episodes.map((ep) => (
              <EpisodeRow
                key={ep.ratingKey}
                episode={ep}
                active={ep.ratingKey === currentRatingKey}
                onClose={onClose}
              />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function EpisodeRow({
  episode,
  active,
  onClose,
}: {
  episode: EpisodeSibling;
  active: boolean;
  onClose: () => void;
}) {
  const img = episode.thumb ? plexImage(episode.thumb, 320, 180) : null;
  const progress =
    episode.viewOffset && episode.duration
      ? Math.min(100, (episode.viewOffset / episode.duration) * 100)
      : null;
  return (
    <li>
      <Link
        href={`/watch/${episode.ratingKey}`}
        onClick={onClose}
        className={`flex gap-3 border-b border-white/5 px-4 py-3 transition-colors last:border-b-0 ${
          active ? "bg-white/10" : "hover:bg-white/5"
        }`}
      >
        <div className="relative aspect-video w-32 shrink-0 overflow-hidden rounded bg-black/50">
          {img && (
            // eslint-disable-next-line @next/next/no-img-element
            <img
              src={img}
              alt=""
              loading="lazy"
              className="h-full w-full object-cover"
            />
          )}
          {progress !== null && (
            <div className="absolute inset-x-0 bottom-0 h-0.5 bg-white/25">
              <div
                className="h-full bg-(--color-accent)"
                style={{ width: `${progress}%` }}
              />
            </div>
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2">
            {episode.index !== undefined && (
              <span className="text-sm font-semibold tabular-nums text-white/85">
                {episode.index}
              </span>
            )}
            <span className="line-clamp-1 text-sm font-medium">
              {episode.title}
            </span>
          </div>
          {episode.summary && (
            <p className="mt-1 line-clamp-2 text-xs text-white/65">
              {episode.summary}
            </p>
          )}
        </div>
      </Link>
    </li>
  );
}

function EpisodesIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="3" y="6" width="18" height="13" rx="2" />
      <line x1="7" y1="3" x2="17" y2="3" />
      <line x1="6" y1="11" x2="11" y2="11" />
      <line x1="6" y1="14" x2="14" y2="14" />
    </svg>
  );
}

function PipIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="2" y="4" width="20" height="14" rx="2" />
      <rect x="12" y="11" width="8" height="6" rx="1" fill="currentColor" />
    </svg>
  );
}

function CaptionsIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <rect x="2" y="5" width="20" height="14" rx="2" />
      <line x1="6" y1="11" x2="10" y2="11" />
      <line x1="14" y1="11" x2="18" y2="11" />
      <line x1="6" y1="15" x2="11" y2="15" />
      <line x1="15" y1="15" x2="18" y2="15" />
    </svg>
  );
}

function LoadingSpinner() {
  return (
    <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center bg-black/40">
      <div className="h-20 w-20 animate-spin rounded-full border-4 border-white/10 border-t-(--color-accent)" />
    </div>
  );
}

function ErrorOverlay({ message }: { message: string }) {
  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-black/85">
      <div className="max-w-md px-6 text-center">
        <p className="mb-2 text-lg font-semibold text-(--color-accent)">
          Playback failed
        </p>
        <p className="text-sm text-white/70">{message}</p>
        <p className="mt-4 text-xs text-white/50">
          Common causes: Plex server unreachable, transcoder busy, or the file
          can&apos;t be HLS direct-streamed.
        </p>
      </div>
    </div>
  );
}

function ProgressBar({
  currentTime,
  duration,
  onSeek,
}: {
  currentTime: number;
  duration: number;
  onSeek: (t: number) => void;
}) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [hovering, setHovering] = useState(false);
  const [scrubbing, setScrubbing] = useState(false);

  const pointToTime = useCallback(
    (clientX: number): number => {
      const track = trackRef.current;
      if (!track || !duration) return 0;
      const rect = track.getBoundingClientRect();
      const ratio = Math.max(
        0,
        Math.min(1, (clientX - rect.left) / rect.width),
      );
      return ratio * duration;
    },
    [duration],
  );

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    setScrubbing(true);
    onSeek(pointToTime(e.clientX));

    const onMove = (ev: PointerEvent) => onSeek(pointToTime(ev.clientX));
    const onUp = () => {
      setScrubbing(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const progress = duration > 0 ? (currentTime / duration) * 100 : 0;
  const expanded = hovering || scrubbing;

  return (
    <div
      ref={trackRef}
      onPointerDown={onPointerDown}
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={() => setHovering(false)}
      role="slider"
      aria-label="Seek"
      aria-valuemin={0}
      aria-valuemax={duration || 0}
      aria-valuenow={currentTime}
      className="group relative cursor-pointer py-2"
    >
      <div
        className={`relative w-full overflow-hidden rounded-full bg-white/30 transition-[height] duration-150 ${
          expanded ? "h-1.5" : "h-1"
        }`}
      >
        <div
          className="absolute inset-y-0 left-0 bg-(--color-accent)"
          style={{ width: `${progress}%` }}
        />
      </div>
      {expanded && (
        <div
          className="absolute top-1/2 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-(--color-accent) shadow-md"
          style={{ left: `${progress}%` }}
        />
      )}
    </div>
  );
}

// ── SVG icons ───────────────────────────────────────────────────────────────

function BackIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="19" y1="12" x2="5" y2="12" />
      <polyline points="12 19 5 12 12 5" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg
      width="28"
      height="28"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <path d="M6 4l14 8-14 8V4z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg
      width="28"
      height="28"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden
    >
      <rect x="6" y="4" width="4" height="16" rx="0.5" />
      <rect x="14" y="4" width="4" height="16" rx="0.5" />
    </svg>
  );
}

function Rewind10Icon() {
  return (
    <svg
      width="26"
      height="26"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M11 4 7 8l4 4" />
      <path d="M7 8h6a6 6 0 1 1-6 6" />
      <text
        x="12"
        y="17"
        textAnchor="middle"
        fontSize="6.5"
        fontWeight="700"
        fill="currentColor"
        stroke="none"
      >
        10
      </text>
    </svg>
  );
}

function Forward10Icon() {
  return (
    <svg
      width="26"
      height="26"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M13 4l4 4-4 4" />
      <path d="M17 8h-6a6 6 0 1 0 6 6" />
      <text
        x="12"
        y="17"
        textAnchor="middle"
        fontSize="6.5"
        fontWeight="700"
        fill="currentColor"
        stroke="none"
      >
        10
      </text>
    </svg>
  );
}

function VolumeIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" fill="currentColor" />
      <path d="M15.5 8.5a4 4 0 0 1 0 7" />
      <path d="M18.5 5.5a8 8 0 0 1 0 13" />
    </svg>
  );
}

function VolumeMutedIcon() {
  return (
    <svg
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" fill="currentColor" />
      <line x1="22" y1="9" x2="16" y2="15" />
      <line x1="16" y1="9" x2="22" y2="15" />
    </svg>
  );
}

function FullscreenIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="4 9 4 4 9 4" />
      <polyline points="20 9 20 4 15 4" />
      <polyline points="4 15 4 20 9 20" />
      <polyline points="20 15 20 20 15 20" />
    </svg>
  );
}

function FullscreenExitIcon() {
  return (
    <svg
      width="22"
      height="22"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="9 4 9 9 4 9" />
      <polyline points="15 4 15 9 20 9" />
      <polyline points="9 20 9 15 4 15" />
      <polyline points="15 20 15 15 20 15" />
    </svg>
  );
}
