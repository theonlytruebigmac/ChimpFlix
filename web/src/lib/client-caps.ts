"use client";

/// Detect what the current browser can decode natively, so the
/// backend's direct-play decision and codec-copy heuristics get an
/// accurate picture instead of the worst-case h264/aac assumption.
///
/// Detection uses TWO signals because they don't always agree:
///
///   * `video.canPlayType(mime)` — what the `<video>` element claims
///     it can decode. Returns "probably" / "maybe" / "". Browsers
///     are notoriously loose with this — Firefox returns "maybe" for
///     HEVC on builds with no working decoder, leading the server to
///     pick a copy path that the browser then silently fails to
///     decode.
///
///   * `MediaSource.isTypeSupported(mime)` — what MSE (the engine
///     HLS.js feeds segments into) can actually demux + decode.
///     This is the authoritative answer for HLS playback because
///     HLS.js never touches `<video src>` directly; it puts every
///     byte through MSE. If MSE says no, the codec will fail in
///     practice no matter what canPlayType claims.
///
/// Both have to agree (probably / true) for a codec to enter the
/// supported list. The cost of being wrong in the optimistic
/// direction is the user seeing an indefinite loading spinner
/// while the server muxes segments the browser can't decode — much
/// worse UX than the small CPU cost of a defensive re-encode.

interface ClientCaps {
  video: string[];
  audio: string[];
  containers: string[];
}

/// Result of one codec probe. `canPlayType` returns "probably" /
/// "maybe" / ""; we map those onto a boolean. `mseSupported` is
/// MediaSource.isTypeSupported's verdict — the binary truth.
function probe(mime: string): { canPlay: boolean; canPlayStrict: boolean; mseSupported: boolean } {
  if (typeof document === "undefined") {
    return { canPlay: false, canPlayStrict: false, mseSupported: false };
  }
  const el = document.createElement("video");
  const result = el.canPlayType(mime);
  const canPlay = result === "probably" || result === "maybe";
  const canPlayStrict = result === "probably";
  let mseSupported = false;
  try {
    mseSupported =
      typeof MediaSource !== "undefined" && MediaSource.isTypeSupported(mime);
  } catch {
    // Some shielded browsers throw on MediaSource access — treat
    // as unsupported.
  }
  return { canPlay, canPlayStrict, mseSupported };
}

/// Detect Safari — the only browser whose HEVC support claims can
/// be trusted in 2026. Detection uses the WebKit-specific quirk
/// that the `safari` substring lives in the UA and `chrome` does
/// not (Chrome on macOS reports "Safari" in its UA *and* "Chrome",
/// so we have to check both).
function isSafari(): boolean {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent.toLowerCase();
  return ua.includes("safari") && !ua.includes("chrome") && !ua.includes("chromium");
}

let cached: ClientCaps | null = null;

export function detectClientCapabilities(): ClientCaps {
  if (cached) return cached;

  // Universal baseline. Even with a broken canPlayType, h264 baseline
  // + aac in mp4/ts works in every browser that can render <video>
  // at all. Direct-play decisions never fail in the pessimistic
  // direction with this floor.
  const video: string[] = ["h264"];
  const audio: string[] = ["aac"];
  const containers: string[] = ["mp4", "ts"];

  // HEVC — only Safari decodes it reliably. Chrome on Windows with
  // hardware HEVC and Firefox on Windows with MFT *can* decode it,
  // but Chrome / Firefox / Edge on Linux all report HEVC support
  // (via both `canPlayType="probably"` AND
  // `MediaSource.isTypeSupported=true`) while the actual decoder
  // either doesn't exist or fails on real Main-profile content.
  // Trusting either API there leads to a silent decode hang —
  // segments stream in, MSE accepts them, no frames appear. The
  // only robust signal in 2026 is user-agent-keyed: assume Safari
  // means real HEVC; assume everyone else is lying. This loses a
  // tiny efficiency win for the small set of Windows browsers with
  // genuine HEVC, but those users still get a clean NVENC h264
  // re-encode that plays fine — vs. the alternative which is the
  // entire title hanging forever.
  if (isSafari()) {
    const p = probe('video/mp4; codecs="hev1.1.6.L93.B0"');
    if (p.canPlayStrict && p.mseSupported) {
      video.push("hevc");
      video.push("h265"); // ffprobe sometimes reports h265 instead
    }
  }

  // VP9 — Chrome / Firefox / Edge. Common for YouTube re-encodes.
  // MSE check matters more here than canPlayType because Firefox
  // claims VP9 support across all OSs but it's hardware-dependent.
  {
    const p = probe('video/webm; codecs="vp09.00.10.08"');
    if (p.canPlay && p.mseSupported) {
      video.push("vp9");
    }
  }

  // AV1 — newer Chrome / Firefox. Library content rarely AV1 today
  // but the dial is starting to move. Same dual-check rationale.
  {
    const p = probe('video/webm; codecs="av01.0.04M.08"');
    if (p.canPlay && p.mseSupported) {
      video.push("av1");
    }
  }

  // AC-3 / E-AC-3 — Safari decodes both natively; Chrome/Firefox
  // don't. Detecting this saves a costly downmix re-encode for the
  // common "Blu-ray rip with AC3 audio" case on a Mac.
  {
    const p = probe('audio/mp4; codecs="ac-3"');
    if (p.canPlay && p.mseSupported) audio.push("ac3");
  }
  {
    const p = probe('audio/mp4; codecs="ec-3"');
    if (p.canPlay && p.mseSupported) audio.push("eac3");
  }

  // Opus in mp4 — Chrome / Firefox / Edge.
  {
    const p = probe('audio/mp4; codecs="opus"');
    if (p.canPlay && p.mseSupported) audio.push("opus");
  }

  // FLAC in mp4 — recent Chrome / Firefox / Edge.
  {
    const p = probe('audio/mp4; codecs="flac"');
    if (p.canPlay && p.mseSupported) audio.push("flac");
  }

  // MP3 — universal but the codec string is its own thing. Add when
  // detected so direct play of MP3-audio-track files (rare but does
  // happen for old DVD rips) works.
  {
    const p = probe("audio/mpeg");
    if (p.canPlay) audio.push("mp3");
  }

  // WebM container — Chrome / Firefox / Edge can demux it. Adding
  // this widens direct-play to .webm sources.
  {
    const p = probe("video/webm");
    if (p.canPlay) containers.push("webm");
  }

  // Matroska container — Chrome and Firefox claim partial support
  // for .mkv, but it's flaky in practice (no native MSE support for
  // most variants). Skip it from the direct-play allow-list; the
  // transcoder is more reliable.

  cached = { video, audio, containers };
  return cached;
}
