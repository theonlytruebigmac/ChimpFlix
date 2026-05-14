import { notFound, redirect } from "next/navigation";
import {
  ChimpFlixPlayer,
  type EpisodeSibling,
  type PlayerMarker,
  type StreamChoice,
} from "@/components/ChimpFlixPlayer";
import {
  ChimpFlixApiError,
  episodes as episodesApi,
  items as itemsApi,
  seasons as seasonsApi,
  type EpisodeDetail,
  type EpisodeListed,
  type ItemDetail,
  type MediaFileSummary,
  type MediaStreamSummary,
  type User,
} from "@/lib/chimpflix-api";
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
    const lang = languageLabel(s.language) ?? "Unknown";
    const codec = s.codec ? ` (${s.codec.toUpperCase()})` : "";
    const channelTag =
      kind === "audio" && s.channels
        ? ` · ${s.channels === 6 ? "5.1" : s.channels === 8 ? "7.1" : `${s.channels}ch`}`
        : "";
    const forcedTag = s.is_forced ? " · forced" : "";
    const defaultTag = s.is_default ? " · default" : "";
    return {
      idx,
      label: `${lang}${codec}${channelTag}${forcedTag}${defaultTag}`,
      language: s.language,
    };
  });
}

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

async function resolveMovie(detail: ItemDetail): Promise<Resolved | null> {
  const file = detail.files[0];
  if (!file) return null;
  const tracks = tracksFor(file);
  return {
    mediaFileId: file.id,
    title: detail.title,
    itemId: detail.id,
    startPositionMs: detail.play_state?.position_ms ?? 0,
    durationMs: file.duration_ms ?? detail.duration_ms ?? undefined,
    backHref: `/?title=${detail.id}`,
    audioTracks: tracks.audio,
    subtitleTracks: tracks.subtitle,
    markers: file.markers ?? [],
  };
}

async function resolveEpisode(
  episode: EpisodeDetail,
  showTitle: string,
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
  return {
    mediaFileId: file.id,
    title: showTitle,
    subtitle: `S${episode.season_number} · E${episode.episode_number} · ${episode.title}`,
    episodeId: episode.id,
    startPositionMs: episode.play_state?.position_ms ?? 0,
    durationMs: file.duration_ms ?? episode.duration_ms ?? undefined,
    backHref: `/?title=${episode.show_id}`,
    nextHref: next ? `/watch/e${next.id}` : undefined,
    nextLabel: next?.title,
    nextThumb: next?.thumb,
    audioTracks: tracks.audio,
    subtitleTracks: tracks.subtitle,
    markers: file.markers ?? [],
    seasonEpisodes,
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
  return resolveEpisode(episodeDetail, detail.title);
}

function parseIndex(v: string | undefined): number | undefined {
  if (!v) return undefined;
  const n = Number.parseInt(v, 10);
  return Number.isFinite(n) && n >= 0 ? n : undefined;
}

/// Pick a 0-indexed track based on the user's saved preference, matching on
/// `language` (ISO 639-2 code). Returns undefined when the preferred
/// language isn't available or the user has no preference set.
function pickByLanguage(
  tracks: StreamChoice[],
  preferredLang: string | null,
): number | undefined {
  if (!preferredLang) return undefined;
  const wanted = preferredLang.toLowerCase();
  const hit = tracks.find((t) => t.language?.toLowerCase() === wanted);
  return hit?.idx;
}

function applyDefaults(
  resolved: Resolved,
  user: User,
  fromQuery: { audio?: number; subtitle?: number },
): { audioIndex?: number; subtitleIndex?: number } {
  const audioIndex =
    fromQuery.audio ??
    pickByLanguage(resolved.audioTracks, user.default_audio_lang);
  const subtitleIndex =
    fromQuery.subtitle ??
    pickByLanguage(resolved.subtitleTracks, user.default_subtitle_lang);
  return { audioIndex, subtitleIndex };
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
      resolved = await resolveEpisode(episode, show.title);
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

  const { audioIndex, subtitleIndex } = applyDefaults(resolved, user, {
    audio: audioFromQuery,
    subtitle: subtitleFromQuery,
  });

  return (
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
      markers={resolved.markers}
      seasonEpisodes={resolved.seasonEpisodes}
    />
  );
}
