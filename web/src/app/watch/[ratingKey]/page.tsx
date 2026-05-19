import { notFound, redirect } from "next/navigation";
import {
  ChimpFlixPlayer,
  type EpisodeSibling,
  type PlayerMarker,
  type StreamChoice,
  type VersionChoice,
} from "@/components/ChimpFlixPlayer";
import {
  ChimpFlixApiError,
  episodes as episodesApi,
  externalSubtitles as externalSubtitlesApi,
  items as itemsApi,
  playState as playStateApi,
  preroll as prerollApi,
  previews as previewsApi,
  seasons as seasonsApi,
  type EpisodeDetail,
  type EpisodeListed,
  type ExternalSubtitle,
  type ItemDetail,
  type MediaFileSummary,
  type MediaStreamSummary,
  type PreviewManifest,
  type SeasonSummary,
  type User,
} from "@/lib/chimpflix-api";
import { PrerollGate } from "@/components/PrerollGate";
import { plexImage } from "@/lib/image";
import { requireUser } from "@/lib/chimpflix-server";

// `ratingKey` is the same identifier the home page produces via
// chimpflix-adapt:
//   - `"<id>"`     — item (movie or show)
//   - `"e<id>"`    — episode (used by Continue Watching for shows)
//
// For a show ratingKey we resolve to the first episode of the first season.
// Continue-Watching-aware resolution lives in the on-deck endpoint already,
// so the typical UX path is to enter /watch via an episode key when an
// episode is in progress.

interface Resolved {
  mediaFileId: number;
  title: string;
  subtitle?: string;
  itemId?: number;
  episodeId?: number;
  startPositionMs: number;
  durationMs?: number;
  backHref: string;
  nextHref?: string;
  nextLabel?: string;
  nextThumb?: string;
  audioTracks: StreamChoice[];
  subtitleTracks: StreamChoice[];
  markers: PlayerMarker[];
  seasonEpisodes?: EpisodeSibling[];
  // ── Episode popup / season switcher ──
  /// Rating-key of the currently-playing episode. The popup uses this
  /// to render the active row in the Netflix-style expanded layout and
  /// to put a checkmark next to the right season in the picker pane.
  currentRatingKey?: string;
  /// DB id of the current season. Used by the picker to show which
  /// season is "checked".
  currentSeasonId?: number;
  /// Show id. Required so the popup can lazy-load episodes for a
  /// different season when the user uses the season picker.
  showId?: number;
  /// Show's display title — header in the season picker pane.
  showTitle?: string;
  /// All seasons in the show, ordered as the API returns them. Source
  /// of truth for the picker list (with a checkmark on the current one).
  seasons?: { id: number; season_number: number; title: string | null }[];
  previewManifest?: PreviewManifest;
  versions?: VersionChoice[];
}

/// Format a media file as a picker label — "4K HDR · HEVC", "1080p",
/// "720p · 12 Mbps". Tries to surface the dimension that distinguishes
/// versions from each other (height bucket + HDR + codec). Falls back
/// to the bitrate when resolution is unknown.
function versionLabel(file: MediaFileSummary): string {
  const parts: string[] = [];
  const h = file.height ?? 0;
  if (h >= 2000) parts.push("4K");
  else if (h >= 1000) parts.push("1080p");
  else if (h >= 700) parts.push("720p");
  else if (h >= 400) parts.push("480p");
  else if (h > 0) parts.push(`${h}p`);
  if (file.hdr_format) parts.push("HDR");
  const videoCodec = file.streams.find((s) => s.kind === "video")?.codec;
  if (videoCodec) parts.push(videoCodec.toUpperCase());
  if (parts.length === 0 && file.bit_rate) {
    parts.push(`${Math.round(file.bit_rate / 1_000_000)} Mbps`);
  }
  return parts.length > 0 ? parts.join(" · ") : `File #${file.id}`;
}

function versionsFor(
  files: MediaFileSummary[],
  external: ExternalSubtitle[],
): VersionChoice[] | undefined {
  if (files.length <= 1) return undefined;
  return files.map((f) => {
    const embedded = streamChoices(f.streams, "subtitle");
    return {
      media_file_id: f.id,
      label: versionLabel(f),
      audioTracks: streamChoices(f.streams, "audio"),
      subtitleTracks: mergeExternalSubtitles(embedded, external),
    };
  });
}

async function maybePreviewManifest(
  mediaFileId: number,
): Promise<PreviewManifest | undefined> {
  // 404 just means previews haven't been generated for this file yet;
  // anything else (network blip, server hiccup) gets the same treatment.
  try {
    return await previewsApi.manifest(mediaFileId);
  } catch {
    return undefined;
  }
}

function languageLabel(code: string | null | undefined): string | null {
  if (!code) return null;
  // ISO 639-2 → human label for the common cases. The frontend doesn't
  // ship a full code table — fall back to the raw code if we don't know.
  const map: Record<string, string> = {
    eng: "English",
    spa: "Spanish",
    fra: "French",
    fre: "French",
    deu: "German",
    ger: "German",
    ita: "Italian",
    por: "Portuguese",
    jpn: "Japanese",
    kor: "Korean",
    chi: "Chinese",
    zho: "Chinese",
    rus: "Russian",
    ara: "Arabic",
    hin: "Hindi",
    nld: "Dutch",
    dut: "Dutch",
    swe: "Swedish",
    nor: "Norwegian",
    dan: "Danish",
    fin: "Finnish",
    pol: "Polish",
    tur: "Turkish",
    vie: "Vietnamese",
    tha: "Thai",
    ind: "Indonesian",
  };
  return map[code.toLowerCase()] ?? code;
}

function streamChoices(
  streams: MediaStreamSummary[],
  kind: "audio" | "subtitle",
): StreamChoice[] {
  const filtered = streams.filter((s) => s.kind === kind);
  return filtered.map((s, idx) => {
    // When the source embeds a track title ("Netflix eng subrip",
    // "SDH eng subrip", "Cantonese (Traditional, Hong Kong) chi
    // subrip"), surface it as the primary label — that's what users
    // see in VLC / mpv / Haruna and it's the only way to tell the
    // half-dozen English subtitle variants of a Bluray remux apart.
    // The language + codec become secondary tags so the user still
    // sees the technical info when scanning the list. When there's
    // no embedded title (most raw transcodes), we fall back to the
    // original "Language (CODEC)" format so we don't end up with a
    // list of "Unknown (SUBRIP)" entries.
    const lang = languageLabel(s.language) ?? "Unknown";
    const codecLabel = s.codec ? s.codec.toUpperCase() : null;
    const channelTag =
      kind === "audio" && s.channels
        ? `${s.channels === 6 ? "5.1" : s.channels === 8 ? "7.1" : `${s.channels}ch`}`
        : null;
    const forcedTag = s.is_forced ? "forced" : null;
    const defaultTag = s.is_default ? "default" : null;

    const title = s.title?.trim();
    let label: string;
    if (title && title.length > 0) {
      // Use embedded title verbatim, with codec / language / dispo
      // tags appended in a dimmer format. Keep the label short — the
      // picker truncates at ~40 chars on narrow screens.
      const extras = [codecLabel, channelTag, forcedTag, defaultTag]
        .filter((t): t is string => !!t)
        .join(" · ");
      label = extras ? `${title} · ${extras}` : title;
    } else {
      const extras = [channelTag, forcedTag, defaultTag]
        .filter((t): t is string => !!t)
        .join(" · ");
      const codec = codecLabel ? ` (${codecLabel})` : "";
      label = extras ? `${lang}${codec} · ${extras}` : `${lang}${codec}`;
    }
    return {
      idx,
      label,
      language: s.language,
      codec: s.codec?.toLowerCase() ?? null,
    };
  });
}

/// Subtitle codec names ffprobe emits for picture-based formats.
/// These need the heavyweight overlay path in the transcoder and
/// the overlay filter is fragile (blocks until the first subtitle
/// frame appears, doesn't compose with ABR / GPU-native pipelines).
/// We never auto-select these — the user can still pick them
/// manually from the picker if they're the only option.
const PICTURE_SUBTITLE_CODECS = new Set([
  "hdmv_pgs_subtitle",
  "pgs",
  "dvd_subtitle",
  "dvdsub",
  "dvb_subtitle",
  "vobsub",
  "xsub",
]);

function tracksFor(file: MediaFileSummary | undefined): {
  audio: StreamChoice[];
  subtitle: StreamChoice[];
} {
  if (!file) return { audio: [], subtitle: [] };
  return {
    audio: streamChoices(file.streams, "audio"),
    subtitle: streamChoices(file.streams, "subtitle"),
  };
}

/// Merge OpenSubtitles-fetched tracks into the picker. Embedded streams
/// keep their numeric `idx`; external rows use `idx = -1` and carry the
/// `externalUrl` flag so the player knows to attach a `<track>` element
/// instead of asking the server to burn it in.
function mergeExternalSubtitles(
  embedded: StreamChoice[],
  external: ExternalSubtitle[],
): StreamChoice[] {
  const externalChoices: StreamChoice[] = external.map((s) => {
    const lang = languageLabel(s.language) ?? s.language ?? "Unknown";
    const tags: string[] = [s.source];
    if (s.forced) tags.push("forced");
    if (s.sdh) tags.push("SDH");
    return {
      idx: -1,
      label: `${lang} · ${tags.join(", ")}`,
      language: s.language,
      externalUrl: `/api/v1/external-subtitles/${s.id}/file`,
    };
  });
  return [...embedded, ...externalChoices];
}

async function resolveMovie(detail: ItemDetail): Promise<Resolved | null> {
  const file = detail.files[0];
  if (!file) return null;
  const tracks = tracksFor(file);
  let external: ExternalSubtitle[] = [];
  try {
    const r = await externalSubtitlesApi.forItem(detail.id);
    external = r.subtitles;
  } catch {
    // Best-effort; an outage in OpenSubtitles shouldn't break playback.
  }
  const previewManifest = await maybePreviewManifest(file.id);
  return {
    mediaFileId: file.id,
    title: detail.title,
    itemId: detail.id,
    startPositionMs: detail.play_state?.position_ms ?? 0,
    durationMs: file.duration_ms ?? detail.duration_ms ?? undefined,
    backHref: `/?title=${detail.id}`,
    audioTracks: tracks.audio,
    subtitleTracks: mergeExternalSubtitles(tracks.subtitle, external),
    markers: file.markers ?? [],
    previewManifest,
    versions: versionsFor(detail.files, external),
  };
}

async function resolveEpisode(
  episode: EpisodeDetail,
  show: { title: string; seasons?: SeasonSummary[] },
): Promise<Resolved | null> {
  const file = episode.files[0];
  if (!file) return null;
  const tracks = tracksFor(file);
  // We fetch the season once for both next-episode resolution and the
  // in-player episode picker so the watch page does at most one extra round
  // trip beyond the episode/show fetches.
  let seasonEpisodesRaw: EpisodeListed[] | undefined;
  try {
    const season = await seasonsApi.get(episode.season_id);
    seasonEpisodesRaw = season.episodes;
  } catch {
    seasonEpisodesRaw = undefined;
  }
  const next = await findNextEpisode(episode, seasonEpisodesRaw);
  const seasonEpisodes: EpisodeSibling[] | undefined = seasonEpisodesRaw?.map(
    (e) => ({
      ratingKey: `e${e.id}`,
      title: e.title,
      thumb: plexImage(e.thumb_path ?? undefined, 320, 180) ?? undefined,
      summary: e.summary ?? undefined,
      duration: e.duration_ms ?? undefined,
      viewOffset: e.play_state?.position_ms,
      index: e.episode_number,
      parentTitle: `Season ${e.season_number}`,
    }),
  );
  const external = await externalSubtitlesApi
    .forEpisode(episode.id)
    .then((r) => r.subtitles)
    .catch(() => [] as ExternalSubtitle[]);
  return {
    mediaFileId: file.id,
    title: show.title,
    subtitle: `S${episode.season_number} · E${episode.episode_number} · ${episode.title}`,
    episodeId: episode.id,
    startPositionMs: episode.play_state?.position_ms ?? 0,
    durationMs: file.duration_ms ?? episode.duration_ms ?? undefined,
    backHref: `/?title=${episode.show_id}`,
    nextHref: next ? `/watch/e${next.id}` : undefined,
    nextLabel: next?.title,
    nextThumb: next?.thumb,
    audioTracks: tracks.audio,
    subtitleTracks: mergeExternalSubtitles(tracks.subtitle, external),
    markers: file.markers ?? [],
    seasonEpisodes,
    currentRatingKey: `e${episode.id}`,
    currentSeasonId: episode.season_id,
    showId: episode.show_id,
    showTitle: show.title,
    seasons: show.seasons?.map((s) => ({
      id: s.id,
      season_number: s.season_number,
      title: s.title,
    })),
    previewManifest: await maybePreviewManifest(file.id),
    versions: versionsFor(episode.files, external),
  };
}

/// Walks within the current season first, then falls back to the first
/// episode of the next season. Returns null at the end of the series.
/// Best-effort: any error swallows and the player just hides the button.
async function findNextEpisode(
  current: EpisodeDetail,
  seasonEpisodes: EpisodeListed[] | undefined,
): Promise<{ id: number; title: string; thumb?: string } | null> {
  try {
    if (seasonEpisodes) {
      const idx = seasonEpisodes.findIndex((e) => e.id === current.id);
      if (idx >= 0 && idx + 1 < seasonEpisodes.length) {
        const ep = seasonEpisodes[idx + 1];
        return {
          id: ep.id,
          title: ep.title,
          thumb: plexImage(ep.thumb_path ?? undefined, 480, 270) ?? undefined,
        };
      }
    }
    const show = await itemsApi.get(current.show_id);
    const sIdx = show.seasons.findIndex((s) => s.id === current.season_id);
    if (sIdx >= 0 && sIdx + 1 < show.seasons.length) {
      const nextSeason = await seasonsApi.get(show.seasons[sIdx + 1].id);
      const first = nextSeason.episodes[0];
      if (first) {
        return {
          id: first.id,
          title: first.title,
          thumb:
            plexImage(first.thumb_path ?? undefined, 480, 270) ?? undefined,
        };
      }
    }
  } catch {
    // Best-effort; the player just won't show a Next button.
  }
  return null;
}

async function resolveShowFirstEpisode(
  detail: ItemDetail,
): Promise<Resolved | null> {
  const firstSeason = detail.seasons[0];
  if (!firstSeason) return null;
  const seasonDetail = await seasonsApi.get(firstSeason.id);
  const firstEpisode = seasonDetail.episodes[0];
  if (!firstEpisode) return null;
  const episodeDetail = await episodesApi.get(firstEpisode.id);
  return resolveEpisode(episodeDetail, detail);
}

function parseIndex(v: string | undefined): number | undefined {
  if (!v) return undefined;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n >= 0 ? n : undefined;
}

/// Pick a track based on the user's saved preference, matching on
/// `language` (ISO 639-2 code). Returns the full StreamChoice so the
/// caller can route an external sub through `externalSubUrl` rather
/// than the negative `idx=-1` placeholder.
function pickByLanguage(
  tracks: StreamChoice[],
  preferredLang: string | null,
): StreamChoice | undefined {
  if (!preferredLang) return undefined;
  const wanted = preferredLang.toLowerCase();
  return tracks.find((t) => t.language?.toLowerCase() === wanted);
}

function applyDefaults(
  resolved: Resolved,
  user: User,
  fromQuery: { audio?: number; subtitle?: number },
): {
  audioIndex?: number;
  subtitleIndex?: number;
  externalSubtitleUrl?: string;
} {
  // Audio: only embedded; idx is sufficient.
  const audioPick = fromQuery.audio !== undefined
    ? undefined
    : pickByLanguage(resolved.audioTracks, user.default_audio_lang);
  const audioIndex = fromQuery.audio ?? audioPick?.idx;

  // Subtitle: an explicit query param wins; otherwise prefer an embedded
  // match (renderable as the burned-in default), then fall back to an
  // external match for the same language.
  if (fromQuery.subtitle !== undefined) {
    return { audioIndex, subtitleIndex: fromQuery.subtitle };
  }
  const preferred = user.default_subtitle_lang;
  if (!preferred) return { audioIndex };
  const wanted = preferred.toLowerCase();
  // Skip picture-based subtitles in auto-selection. The transcoder's
  // overlay-burn path for PGS/DVD subs blocks ffmpeg until the first
  // subtitle frame appears (a quirk of the overlay filter), so a
  // user landing on a movie whose PGS subs don't start for 30-60 s
  // sees a frozen loading spinner. External text subs (WebVTT from
  // OpenSubtitles) handle the same content without the overlay
  // pipeline. Users can still pick PGS manually from the captions
  // menu — this only changes the default.
  const embedded = resolved.subtitleTracks.find(
    (t) =>
      !t.externalUrl
      && t.language?.toLowerCase() === wanted
      && !PICTURE_SUBTITLE_CODECS.has(t.codec ?? ""),
  );
  if (embedded) return { audioIndex, subtitleIndex: embedded.idx };
  const external = resolved.subtitleTracks.find(
    (t) => t.externalUrl && t.language?.toLowerCase() === wanted,
  );
  if (external?.externalUrl) {
    return { audioIndex, externalSubtitleUrl: external.externalUrl };
  }
  return { audioIndex };
}

export default async function WatchPage({
  params,
  searchParams,
}: {
  params: Promise<{ ratingKey: string }>;
  searchParams: Promise<{ audio?: string; subtitle?: string }>;
}) {
  const { ratingKey } = await params;
  const { audio, subtitle } = await searchParams;
  const audioFromQuery = parseIndex(audio);
  const subtitleFromQuery = parseIndex(subtitle);
  const user = await requireUser(`/watch/${ratingKey}`);

  let resolved: Resolved | null = null;
  try {
    if (ratingKey.startsWith("e")) {
      const epId = Number.parseInt(ratingKey.slice(1), 10);
      if (!Number.isFinite(epId) || epId <= 0) notFound();
      const episode = await episodesApi.get(epId);
      const show = await itemsApi.get(episode.show_id);
      resolved = await resolveEpisode(episode, show);
    } else {
      const id = Number.parseInt(ratingKey, 10);
      if (!Number.isFinite(id) || id <= 0) notFound();
      const detail = await itemsApi.get(id);
      if (detail.kind === "movie") {
        resolved = await resolveMovie(detail);
      } else if (detail.kind === "show") {
        resolved = await resolveShowFirstEpisode(detail);
      }
    }
  } catch (e) {
    if (e instanceof ChimpFlixApiError) {
      if (e.status === 401) {
        redirect(
          `/login?next=${encodeURIComponent(`/watch/${ratingKey}`)}`,
        );
      }
      if (e.status === 404) notFound();
    }
    throw e;
  }

  if (!resolved) notFound();

  const { audioIndex, subtitleIndex, externalSubtitleUrl } = applyDefaults(
    resolved,
    user,
    { audio: audioFromQuery, subtitle: subtitleFromQuery },
  );

  // Pull the server-configured played threshold so the player auto-
  // scrobbles at the operator's chosen percentage instead of a baked
  // 90%. On error (e.g. brief network blip) fall back to 90 so the
  // page still renders rather than hard-failing the watch flow.
  let playedThresholdPct = 90;
  let completionBehaviour: string = "threshold_pct";
  try {
    const cfg = await playStateApi.config();
    playedThresholdPct = cfg.played_threshold_pct;
    completionBehaviour = cfg.completion_behaviour;
  } catch {
    // Keep defaults.
  }

  // Check whether a pre-roll is configured + enabled. Fetched here on
  // the server so the wrapper can decide synchronously whether to
  // gate the player. Failures fall through to no-preroll.
  //
  // Suppressed on resume: pre-roll is a session-start sting, not
  // something to replay every time the viewer comes back to a paused
  // show. Without this guard, hitting "skip" on the pre-roll lands the
  // viewer at their saved position, which feels like the player just
  // skipped forward by 10 minutes.
  let prerollUrl: string | null = null;
  let prerollVolume = 100;
  const isResume = (resolved.startPositionMs ?? 0) > 30_000;
  if (!isResume) {
    try {
      const ps = await prerollApi.status();
      if (ps.enabled && ps.url) {
        prerollUrl = ps.url;
        prerollVolume = ps.volume ?? 100;
      }
    } catch {
      // No-op.
    }
  }

  const player = (
    <ChimpFlixPlayer
      title={resolved.title}
      subtitle={resolved.subtitle}
      mediaFileId={resolved.mediaFileId}
      durationMs={resolved.durationMs}
      startPositionMs={resolved.startPositionMs}
      itemId={resolved.itemId}
      episodeId={resolved.episodeId}
      backHref={resolved.backHref}
      nextHref={resolved.nextHref}
      nextLabel={resolved.nextLabel}
      nextThumb={resolved.nextThumb}
      audioTracks={resolved.audioTracks}
      subtitleTracks={resolved.subtitleTracks}
      audioIndex={audioIndex}
      subtitleIndex={subtitleIndex}
      externalSubtitleUrl={externalSubtitleUrl}
      markers={resolved.markers}
      seasonEpisodes={resolved.seasonEpisodes}
      currentRatingKey={resolved.currentRatingKey}
      currentSeasonId={resolved.currentSeasonId}
      showId={resolved.showId}
      showTitle={resolved.showTitle}
      seasons={resolved.seasons}
      previewManifest={resolved.previewManifest}
      versions={resolved.versions}
      playedThresholdPct={playedThresholdPct}
      completionBehaviour={completionBehaviour}
    />
  );

  if (prerollUrl) {
    return (
      <PrerollGate prerollUrl={prerollUrl} prerollVolume={prerollVolume}>
        {player}
      </PrerollGate>
    );
  }
  return player;
}
