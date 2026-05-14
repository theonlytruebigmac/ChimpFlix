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
import {
  ChimpFlixApiError,
  stream as streamApi,
  playState as playStateApi,
} from "@/lib/chimpflix-api";
import { getPrefs, updatePrefs, usePrefs } from "@/lib/prefs";

export interface StreamChoice {
  // 0-indexed among that kind's streams in the file. Pass straight to the
  // server's audio_index / subtitle_index.
  idx: number;
  label: string;
  language?: string | null;
}

export interface PlayerMarker {
  kind: "intro" | "credits" | string;
  start_ms: number;
  end_ms: number;
}

export type EpisodeSibling = {
  ratingKey: string;
  title: string;
  thumb?: string;
  summary?: string;
  duration?: number;
  viewOffset?: number;
  index?: number;
  parentTitle?: string;
};

interface Props {
  title: string;
  subtitle?: string;
  mediaFileId: number;
  // Best-known duration in milliseconds. Comes from the file's metadata
  // (ffprobe) — authoritative across the whole title, unlike `video.duration`
  // which only reflects what HLS has surfaced so far. Used for the time
  // display and progress bar.
  durationMs?: number;
  startPositionMs?: number;
  itemId?: number;
  episodeId?: number;
  backHref: string;
  nextHref?: string;
  nextLabel?: string;
  nextThumb?: string;
  audioTracks?: StreamChoice[];
  subtitleTracks?: StreamChoice[];
  audioIndex?: number;
  subtitleIndex?: number;
  markers?: PlayerMarker[];
  seasonEpisodes?: EpisodeSibling[];
}

const PLAY_STATE_INTERVAL_MS = 10_000;
const SCROBBLE_THRESHOLD = 0.9;
const COUNTDOWN_WINDOW_SECONDS = 10;
const SPEED_OPTIONS = [0.5, 0.75, 1, 1.25, 1.5, 2] as const;

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

function activeMarker(
  currentMs: number,
  markers?: PlayerMarker[],
): PlayerMarker | null {
  if (!markers) return null;
  for (const m of markers) {
    if (currentMs >= m.start_ms && currentMs <= m.end_ms) return m;
  }
  return null;
}

function markerLabel(m: PlayerMarker): string {
  if (m.kind === "credits") return "Skip Credits";
  if (m.kind === "commercial") return "Skip Ad";
  return "Skip Intro";
}

export function ChimpFlixPlayer({
  title,
  subtitle,
  mediaFileId,
  durationMs,
  startPositionMs = 0,
  itemId,
  episodeId,
  backHref,
  nextHref,
  nextLabel,
  nextThumb,
  audioTracks,
  subtitleTracks,
  audioIndex,
  subtitleIndex,
  markers,
  seasonEpisodes,
}: Props) {
  const router = useRouter();
  const [prefs] = usePrefs();
  const containerRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const hideTimerRef = useRef<number | null>(null);
  const hlsRef = useRef<Hls | null>(null);
  const scrobbledRef = useRef(false);
  // Captured the resume position so a track switch mid-playback comes back
  // to roughly where the user was, not the original startPositionMs.
  const liveTimeMsRef = useRef<number>(startPositionMs);

  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(startPositionMs / 1000);
  // Server-provided duration is the source of truth for the time display.
  // `video.duration` only reflects what HLS has parsed so far, which on a
  // live transcoder is wildly under-counted (e.g. 15 minutes for a 2-hour
  // movie). Fall back to that only if the server didn't give us one.
  const [videoDuration, setVideoDuration] = useState(
    durationMs ? durationMs / 1000 : 0,
  );
  const [muted, setMuted] = useState(false);
  const [volume, setVolume] = useState(1);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showControls, setShowControls] = useState(true);
  const [autoplayBlocked, setAutoplayBlocked] = useState(false);
  // Local selection state. `undefined` = transcoder default. For subtitles
  // we use `null` to mean "explicitly off" (no subtitle_index sent).
  const [audioSel, setAudioSel] = useState<number | undefined>(audioIndex);
  const [subtitleSel, setSubtitleSel] = useState<number | null | undefined>(
    subtitleIndex,
  );
  const [tracksOpen, setTracksOpen] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [speedOpen, setSpeedOpen] = useState(false);
  const [autoNextCancelled, setAutoNextCancelled] = useState(false);
  const [episodesOpen, setEpisodesOpen] = useState(false);
  const [pipActive, setPipActive] = useState(false);
  const [showRemaining, setShowRemaining] = useState(true);
  // Derived: the marker (if any) that contains the current playback time.
  const activeMarkerOverlay = activeMarker(currentTime * 1000, markers);

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

  // ── Session setup ────────────────────────────────────────────────────────
  // Asks the Rust backend for a play session, wires up HTML5 <video> for
  // direct play or hls.js for transcode, and tears the session down on
  // unmount. Re-runs when audio/subtitle selection changes (a fresh manifest
  // is required because the transcoder burns subtitles into the video).
  useEffect(() => {
    let cancelled = false;
    let sessionId: string | null = null;
    let cleanup: () => void = () => {};

    const resumeMs =
      liveTimeMsRef.current > 1000 ? liveTimeMsRef.current : startPositionMs;

    async function start() {
      const video = videoRef.current;
      if (!video) return;
      setLoading(true);
      setError(null);

      let resp;
      try {
        resp = await streamApi.createSession({
          media_file_id: mediaFileId,
          start_position_ms: resumeMs,
          audio_index: audioSel,
          subtitle_index: subtitleSel === null ? undefined : subtitleSel,
          client: {
            supported_video_codecs: ["h264"],
            supported_audio_codecs: ["aac"],
            supported_containers: ["mp4", "ts"],
          },
        });
      } catch (e) {
        if (cancelled) return;
        if (e instanceof ChimpFlixApiError && e.status === 401) {
          router.push(
            "/login?next=" + encodeURIComponent(window.location.pathname),
          );
          return;
        }
        setError("Could not start playback");
        return;
      }

      // Assign sessionId BEFORE checking `cancelled`. If the user navigated
      // away during the create-session round-trip, cleanup needs to know
      // the id so it can DELETE the orphan transcoder — checking cancelled
      // first would leak the session.
      sessionId = resp.session.id !== "direct" ? resp.session.id : null;

      if (cancelled) return;
      if (resp.session.duration_ms) {
        setVideoDuration(resp.session.duration_ms / 1000);
      }

      function applyResume() {
        if (!video) return;
        if (resumeMs > 1000) {
          video.currentTime = resumeMs / 1000;
        }
      }

      if (resp.session.mode === "direct" && resp.session.direct_url) {
        video.src = resp.session.direct_url;
        const onLoaded = () => {
          applyResume();
          attemptPlay();
        };
        video.addEventListener("loadedmetadata", onLoaded, { once: true });
        cleanup = () => video.removeEventListener("loadedmetadata", onLoaded);
        return;
      }

      if (resp.session.mode === "transcode" && resp.session.hls_master_url) {
        const url = resp.session.hls_master_url;
        if (video.canPlayType("application/vnd.apple.mpegurl")) {
          video.src = url;
          const onLoaded = () => {
            applyResume();
            attemptPlay();
          };
          video.addEventListener("loadedmetadata", onLoaded, { once: true });
          cleanup = () =>
            video.removeEventListener("loadedmetadata", onLoaded);
          return;
        }
        try {
          const HlsModule = (await import("hls.js")).default;
          if (cancelled) return;
          if (HlsModule.isSupported()) {
            const hls = new HlsModule({
              enableWorker: true,
              backBufferLength: 30,
              manifestLoadingTimeOut: 6000,
              manifestLoadingMaxRetry: 2,
              levelLoadingTimeOut: 6000,
              fragLoadingTimeOut: 20000,
              abrEwmaDefaultEstimate: 5_000_000,
            });
            hlsRef.current = hls;
            hls.loadSource(url);
            hls.attachMedia(video);
            hls.on(HlsModule.Events.MANIFEST_PARSED, () => {
              applyResume();
              attemptPlay();
            });
            hls.on(HlsModule.Events.ERROR, (_e, data) => {
              if (data.fatal) {
                setError(`${data.type} / ${data.details}`);
              }
            });
            cleanup = () => {
              hlsRef.current = null;
              hls.destroy();
            };
            return;
          }
        } catch (e) {
          setError(e instanceof Error ? e.message : String(e));
          return;
        }
        setError("HLS not supported in this browser");
        return;
      }

      setError("Server returned an unplayable session");
    }

    // Also fire the DELETE on `pagehide` (tab close, navigation away from
    // the SPA) — React's unmount cleanup can race with the browser
    // tearing down the page and a normal `fetch` gets aborted. The
    // `keepalive: true` flag tells the browser to let the request
    // outlive the page.
    function teardownSession() {
      if (!sessionId) return;
      try {
        fetch(`/api/v1/stream/sessions/${encodeURIComponent(sessionId)}`, {
          method: "DELETE",
          keepalive: true,
          credentials: "include",
        }).catch(() => {});
      } catch {
        // Fetch can throw synchronously during unload on some browsers —
        // we tried, the server-side reaper will mop up either way.
      }
    }
    window.addEventListener("pagehide", teardownSession);

    start();

    return () => {
      cancelled = true;
      cleanup();
      window.removeEventListener("pagehide", teardownSession);
      if (sessionId) {
        // Use the same keepalive path: even React unmounts can coincide
        // with the page going away (Back button after watching).
        teardownSession();
      }
    };
    // Re-running on audio/subtitle changes tears down the existing session
    // and asks for a new one with the chosen tracks. `startPositionMs` is
    // intentionally captured once via liveTimeMsRef so a deps change here
    // doesn't restart playback from the original resume point.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mediaFileId, audioSel, subtitleSel]);

  // ── Video state subscriptions ────────────────────────────────────────────
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const onLoadedMetadata = () => {
      // Only adopt video.duration if the server didn't give us one. Server
      // metadata is authoritative for HLS where video.duration grows over
      // time as segments arrive.
      if (!durationMs && Number.isFinite(video.duration)) {
        setVideoDuration(video.duration);
      }
    };
    const onTimeUpdate = () => {
      setCurrentTime(video.currentTime);
      liveTimeMsRef.current = Math.floor(video.currentTime * 1000);
    };
    const onPlay = () => {
      setPlaying(true);
      setAutoplayBlocked(false);
    };
    const onPause = () => setPlaying(false);
    const onWaiting = () => setLoading(true);
    const onCanPlay = () => {
      setLoading(false);
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
  }, [attemptPlay, autoplayBlocked, durationMs]);

  // Apply persisted prefs (volume, muted, playback rate) on mount.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const saved = getPrefs();
    video.volume = saved.volume;
    video.muted = saved.muted;
    video.playbackRate = saved.playbackRate;
    setPlaybackRate(saved.playbackRate);
  }, []);

  // Fullscreen tracking.
  useEffect(() => {
    const onChange = () =>
      setIsFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  // PiP tracking.
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

  // Periodic play-state updates + scrobble at threshold.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    function report() {
      if (!video || video.paused || video.ended) return;
      const positionMs = Math.floor(video.currentTime * 1000);
      const knownDurationMs =
        videoDuration > 0
          ? Math.floor(videoDuration * 1000)
          : Number.isFinite(video.duration)
            ? Math.floor(video.duration * 1000)
            : undefined;
      playStateApi
        .update({
          item_id: itemId,
          episode_id: episodeId,
          position_ms: positionMs,
          duration_ms: knownDurationMs,
        })
        .catch(() => {});
      if (
        !scrobbledRef.current &&
        knownDurationMs &&
        positionMs / knownDurationMs >= SCROBBLE_THRESHOLD
      ) {
        scrobbledRef.current = true;
        playStateApi
          .scrobble({ item_id: itemId, episode_id: episodeId })
          .catch(() => {});
      }
    }

    const interval = window.setInterval(report, PLAY_STATE_INTERVAL_MS);
    const onPause = () => report();
    const onEnded = () => report();
    video.addEventListener("pause", onPause);
    video.addEventListener("ended", onEnded);
    return () => {
      window.clearInterval(interval);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("ended", onEnded);
      report();
    };
  }, [itemId, episodeId, videoDuration]);

  // Auto-advance to next episode.
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

  // Idle-hide controls.
  const resetHide = useCallback(() => {
    setShowControls(true);
    if (hideTimerRef.current) window.clearTimeout(hideTimerRef.current);
    hideTimerRef.current = window.setTimeout(() => {
      const v = videoRef.current;
      if (v && !v.paused) setShowControls(false);
    }, 3000);
  }, []);

  // Imperative controls.
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
      // PiP can be blocked or unsupported — best-effort.
    }
  }, []);

  // Audio/subtitle selection causes a fresh session (the transcoder burns
  // subtitles in, so there's no in-stream switch).
  const selectAudio = useCallback((idx: number) => {
    setAudioSel(idx);
  }, []);

  const selectSubtitle = useCallback((idx: number | null) => {
    // null = explicitly off; we send no subtitle_index to the server.
    setSubtitleSel(idx);
  }, []);

  const toggleSubtitles = useCallback(() => {
    if (subtitleSel === null || subtitleSel === undefined) {
      const first = subtitleTracks?.[0]?.idx;
      if (first !== undefined) selectSubtitle(first);
    } else {
      selectSubtitle(null);
    }
  }, [subtitleSel, subtitleTracks, selectSubtitle]);

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

  // Keyboard shortcuts.
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
        className="h-full w-full bg-black"
      />

      {error && <ErrorOverlay message={error} />}
      {loading && !error && !autoplayBlocked && <LoadingSpinner />}
      {autoplayBlocked && !error && <BigPlayButton onClick={attemptPlay} />}

      {activeMarkerOverlay && (
        <button
          type="button"
          onClick={() => seekTo(activeMarkerOverlay.end_ms / 1000)}
          className="pointer-events-auto absolute bottom-32 right-8 z-30 rounded-md border border-white/30 bg-white/95 px-6 py-2.5 text-sm font-semibold text-black shadow-2xl transition-all hover:scale-[1.03] hover:bg-white"
        >
          {markerLabel(activeMarkerOverlay)}
        </button>
      )}

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
        {/* Top bar — minimal: back affordance only. */}
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

        {/* Bottom controls. */}
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
                  onToggle={() => setEpisodesOpen((o) => !o)}
                  onClose={() => setEpisodesOpen(false)}
                />
              )}
              {(((audioTracks?.length ?? 0) > 1) ||
                ((subtitleTracks?.length ?? 0) > 0)) && (
                <TracksControl
                  audioTracks={audioTracks ?? []}
                  subtitleTracks={subtitleTracks ?? []}
                  audioSel={audioSel}
                  subtitleSel={subtitleSel}
                  open={tracksOpen}
                  onToggle={() => setTracksOpen((o) => !o)}
                  onClose={() => setTracksOpen(false)}
                  onAudioSelect={selectAudio}
                  onSubtitleSelect={selectSubtitle}
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
                aria-label={
                  pipActive ? "Exit picture-in-picture" : "Picture-in-picture"
                }
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
  audioTracks,
  subtitleTracks,
  audioSel,
  subtitleSel,
  open,
  onToggle,
  onClose,
  onAudioSelect,
  onSubtitleSelect,
}: {
  audioTracks: StreamChoice[];
  subtitleTracks: StreamChoice[];
  audioSel?: number;
  subtitleSel?: number | null;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onAudioSelect: (idx: number) => void;
  onSubtitleSelect: (idx: number | null) => void;
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
            tracks={audioTracks}
            currentIdx={audioSel}
            offSelected={false}
            onSelect={(idx) => {
              if (idx !== null) onAudioSelect(idx);
            }}
            offOption={false}
          />
          <StreamColumn
            label="Subtitles"
            tracks={subtitleTracks}
            currentIdx={subtitleSel === null ? undefined : subtitleSel}
            offSelected={subtitleSel === null}
            onSelect={(idx) => onSubtitleSelect(idx)}
            offOption={true}
          />
        </div>
      )}
    </div>
  );
}

function StreamColumn({
  label,
  tracks,
  currentIdx,
  offSelected,
  onSelect,
  offOption,
}: {
  label: string;
  tracks: StreamChoice[];
  currentIdx?: number;
  offSelected: boolean;
  onSelect: (idx: number | null) => void;
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
            active={offSelected}
            onClick={() => onSelect(null)}
          />
        )}
        {tracks.map((t) => (
          <TrackRow
            key={t.idx}
            label={t.label}
            active={currentIdx === t.idx}
            onClick={() => onSelect(t.idx)}
          />
        ))}
        {tracks.length === 0 && !offOption && (
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
  return (
    <div className="pointer-events-auto absolute bottom-28 right-8 z-30 w-80 overflow-hidden rounded-md border border-white/10 bg-black/95 shadow-2xl backdrop-blur-sm">
      {thumb && (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={thumb}
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
    const onMove = (ev: PointerEvent) =>
      onVolumeChange(pointToVolume(ev.clientX));
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

function EpisodesControl({
  open,
  episodes,
  onToggle,
  onClose,
}: {
  open: boolean;
  episodes: EpisodeSibling[];
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
              <EpisodeRow key={ep.ratingKey} episode={ep} onClose={onClose} />
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function EpisodeRow({
  episode,
  onClose,
}: {
  episode: EpisodeSibling;
  onClose: () => void;
}) {
  const progress =
    episode.viewOffset && episode.duration
      ? Math.min(100, (episode.viewOffset / episode.duration) * 100)
      : null;
  return (
    <li>
      <Link
        href={`/watch/${episode.ratingKey}`}
        onClick={onClose}
        className="flex gap-3 border-b border-white/5 px-4 py-3 transition-colors last:border-b-0 hover:bg-white/5"
      >
        <div className="relative aspect-video w-32 shrink-0 overflow-hidden rounded bg-black/50">
          {episode.thumb && (
            // eslint-disable-next-line @next/next/no-img-element
            <img
              src={episode.thumb}
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
          Common causes: server unreachable, transcoder busy, or the file
          can&apos;t be HLS-streamed.
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
