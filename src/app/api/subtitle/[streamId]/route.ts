import { NextRequest, NextResponse } from "next/server";
import { plexServer } from "@/lib/plex";
import { resolveServerAuth } from "@/lib/session";

// Fetches a Plex subtitle stream as WebVTT so the player's <track> element
// can render it natively.
//
// Plex's universal subtitle transcoder (the only endpoint that reliably
// serves subtitle content across server versions) returns ASS/SSA format,
// not SRT or VTT. The <track> element only speaks WebVTT, so we convert
// ASS->VTT inline. This is the same endpoint Plex Web uses; we just have to
// ask for `Accept: */*` (Plex refuses to serve subtitle text when the
// client demands JSON) and provide a path + subtitleStreamID.
export async function GET(
  req: NextRequest,
  ctx: { params: Promise<{ streamId: string }> },
) {
  try {
    let auth;
    try {
      auth = await resolveServerAuth();
    } catch (err) {
      return NextResponse.json(
        {
          error: `auth resolution threw: ${err instanceof Error ? err.message : String(err)}`,
        },
        { status: 500 },
      );
    }
    if (!auth) {
      return NextResponse.json({ error: "unauthorized" }, { status: 401 });
    }

    const { streamId } = await ctx.params;
    const id = Number(streamId);
    if (!Number.isFinite(id) || id <= 0) {
      return NextResponse.json({ error: "bad streamId" }, { status: 400 });
    }
    const ratingKeyStr = req.nextUrl.searchParams.get("ratingKey");
    const ratingKey = Number(ratingKeyStr);
    if (!ratingKeyStr || !Number.isFinite(ratingKey) || ratingKey <= 0) {
      return NextResponse.json(
        { error: "missing or bad ratingKey" },
        { status: 400 },
      );
    }

    // Mirror Plex Web's URL exactly. X-Plex-Features in particular is what
    // tells the subtitle transcoder "client can play external subtitle
    // files directly, serve raw text" — without it Plex falls back to
    // returning the HLS master playlist instead.
    const params = new URLSearchParams({
      hasMDE: "1",
      path: `/library/metadata/${ratingKey}`,
      mediaIndex: "0",
      partIndex: "0",
      protocol: "dash",
      fastSeek: "1",
      directPlay: "0",
      directStream: "1",
      subtitleSize: "100",
      audioBoost: "100",
      autoAdjustSubtitle: "1",
      mediaBufferSize: "102400",
      subtitleStreamID: String(id),
      subtitles: "auto",
      session: generateSessionId(),
      "X-Plex-Features": "external-media,indirect-media,hub-style-list",
      "X-Plex-Incomplete-Segments": "1",
    });

    let upstream: Response;
    try {
      upstream = await plexServer(
        `/video/:/transcode/universal/subtitles?${params}`,
        auth,
        {
          headers: {
            // Plex's subtitle transcoder refuses to serve text content when
            // JSON is demanded; ask for anything.
            Accept: "*/*",
          },
        },
      );
    } catch (err) {
      return NextResponse.json(
        {
          error: `plex fetch threw: ${err instanceof Error ? err.message : String(err)}`,
        },
        { status: 502 },
      );
    }
    if (!upstream.ok) {
      return NextResponse.json(
        { error: `plex returned ${upstream.status}` },
        { status: upstream.status === 404 ? 404 : 502 },
      );
    }

    // Guard against gigantic bodies. A subtitle file is < 1 MB even for a
    // long episode; anything larger is Plex misinterpreting our request.
    const contentLength = Number(upstream.headers.get("content-length") ?? 0);
    if (contentLength > 5_000_000) {
      return NextResponse.json(
        {
          error: `plex returned ${contentLength} bytes (too large for a subtitle)`,
          contentType: upstream.headers.get("content-type"),
        },
        { status: 502 },
      );
    }

    const body = await upstream.text();
    if (!body || body.trim().length === 0) {
      return NextResponse.json(
        { error: "plex returned empty subtitle body" },
        { status: 502 },
      );
    }

    // Plex's DASH subtitle output is typically ASS. Detect by header line
    // and convert; fall through to other formats unchanged.
    let vtt: string;
    if (looksLikeAss(body)) {
      vtt = assToVtt(body);
    } else if (looksLikeWebVtt(body)) {
      vtt = body;
    } else {
      // SRT or some other timecode-based format we can at least try.
      vtt = srtToVtt(body);
    }

    return new Response(vtt, {
      status: 200,
      headers: {
        "Content-Type": "text/vtt; charset=utf-8",
        "Cache-Control": "private, max-age=3600",
      },
    });
  } catch (err) {
    return NextResponse.json(
      {
        error: `unhandled: ${err instanceof Error ? err.message : String(err)}`,
        stack: err instanceof Error ? err.stack?.split("\n").slice(0, 6) : null,
      },
      { status: 500 },
    );
  }
}

function generateSessionId(): string {
  // Match Plex Web's session ID shape: 24-ish chars, lowercase alphanumeric.
  // Doesn't need to be cryptographically strong — Plex uses it as an opaque
  // session key, not for auth.
  const rand = () => Math.random().toString(36).slice(2, 14);
  return `${rand()}${rand()}`;
}

function looksLikeAss(text: string): boolean {
  return /^\s*\[Script Info\]/i.test(text);
}

function looksLikeWebVtt(text: string): boolean {
  return /^﻿?WEBVTT(\s|$)/.test(text);
}

// SRT -> VTT: same shape, just swap the timestamp separator (`,` for `.`)
// and add a WEBVTT header.
function srtToVtt(srt: string): string {
  const normalized = srt.replace(/\r\n?/g, "\n");
  const fixed = normalized.replace(
    /(\d{1,2}:\d{2}:\d{2}),(\d{3})/g,
    "$1.$2",
  );
  return `WEBVTT\n\n${fixed}`;
}

// ASS/SSA -> VTT. Walks the [Events] section and emits one VTT cue per
// Dialogue line. Drops [Script Info], [V4+ Styles], and any other section
// blocks since they have no WebVTT equivalent. ASS override codes ({\an8},
// {\b1}, etc.) and \N newlines get normalized.
function assToVtt(ass: string): string {
  const out: string[] = ["WEBVTT", ""];
  let inEvents = false;
  let formatFields: string[] | null = null;
  let cueNum = 0;

  for (const raw of ass.split(/\r?\n/)) {
    const line = raw.trimEnd();
    if (/^\[Events\]/i.test(line)) {
      inEvents = true;
      continue;
    }
    if (/^\[[^\]]+\]/.test(line)) {
      inEvents = false;
      continue;
    }
    if (!inEvents) continue;

    // First line in [Events] is `Format: field1, field2, ...` describing
    // the column layout. We need it to know which columns are Start, End,
    // and Text — the order isn't guaranteed.
    if (/^Format:/i.test(line)) {
      formatFields = line
        .slice("Format:".length)
        .split(",")
        .map((s) => s.trim().toLowerCase());
      continue;
    }
    if (!/^Dialogue:/i.test(line)) continue;
    if (!formatFields) continue;

    const value = line.slice("Dialogue:".length).trim();
    // Split on the first N-1 commas so the Text field — which can itself
    // contain commas — stays intact.
    const parts: string[] = [];
    let remaining = value;
    for (let i = 0; i < formatFields.length - 1; i++) {
      const comma = remaining.indexOf(",");
      if (comma < 0) break;
      parts.push(remaining.slice(0, comma).trim());
      remaining = remaining.slice(comma + 1);
    }
    parts.push(remaining); // last field is Text, keeps any commas

    const startIdx = formatFields.indexOf("start");
    const endIdx = formatFields.indexOf("end");
    const textIdx = formatFields.indexOf("text");
    if (startIdx < 0 || endIdx < 0 || textIdx < 0) continue;

    const start = assTimeToVtt(parts[startIdx]?.trim());
    const end = assTimeToVtt(parts[endIdx]?.trim());
    const text = cleanAssText(parts[textIdx] ?? "");
    if (!start || !end || !text) continue;

    cueNum++;
    out.push(String(cueNum));
    out.push(`${start} --> ${end}`);
    out.push(text);
    out.push("");
  }
  return out.join("\n");
}

// ASS uses `H:MM:SS.cc` (centiseconds — only two digits after the dot).
// VTT requires `HH:MM:SS.fff` (milliseconds — three digits).
function assTimeToVtt(time: string | undefined): string | null {
  if (!time) return null;
  const m = /^(\d+):(\d{2}):(\d{2})\.(\d{1,3})$/.exec(time);
  if (!m) return null;
  const [, h, mm, ss, frac] = m;
  // Centiseconds (cc) need a trailing 0 to become milliseconds.
  const ms = frac.padEnd(3, "0").slice(0, 3);
  return `${h.padStart(2, "0")}:${mm}:${ss}.${ms}`;
}

function cleanAssText(text: string): string {
  return text
    .replace(/\\N/g, "\n") // ASS hard line break
    .replace(/\\n/g, "\n") // soft line break (some renderers)
    .replace(/\\h/g, " ") // ASS hard space
    .replace(/\{[^}]*\}/g, "") // strip override codes like {\an8}, {\b1}
    .trim();
}
