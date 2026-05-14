export type CastMember = { name: string; role?: string };

export type Marker = {
  type: string; // "intro", "credits", "commercial"
  startMs: number;
  endMs: number;
};

// A single Plex audio or subtitle stream. We keep just enough to label it in
// the picker UI and pass its ID back to Plex's transcoder. `language` is the
// human label ("English"), `languageCode` is the ISO 639-2 short code
// ("eng") which is what we match against user prefs.
export type MediaStream = {
  id: number;
  language?: string;
  languageCode?: string;
  codec?: string;
  displayTitle?: string;
  forced?: boolean;
  default?: boolean;
  selected?: boolean;
};

export type MediaItem = {
  ratingKey: string;
  key: string;
  type: string;
  title: string;
  summary?: string;
  thumb?: string;
  art?: string;
  year?: number;
  duration?: number;
  viewOffset?: number;
  contentRating?: string;
  rating?: number;
  genres?: string[];
  parentTitle?: string;
  parentRatingKey?: string;
  grandparentTitle?: string;
  grandparentRatingKey?: string;
  index?: number;
  cast?: CastMember[];
  directors?: string[];
  writers?: string[];
  childCount?: number;
  leafCount?: number;
  // Millisecond epoch of when the item was added to the library.
  // Card uses this to render the red "Recently Added" / "New Season"
  // ribbon when within the recency window.
  addedAt?: number;
  markers?: Marker[];
  librarySectionID?: string;
  // Populated only when the source Plex response includes Media/Part/Stream
  // (i.e. single-item metadata fetches, not the slimmed list endpoints).
  audioStreams?: MediaStream[];
  subtitleStreams?: MediaStream[];
  // ID of the first Part in the first Media block. Needed to hit Plex's
  // /library/parts/<partId> endpoints (subtitle download, file info). Plex
  // exposes it on every leaf metadata item.
  partId?: number;
};

export type Section = {
  key: string;
  title: string;
  type: string;
};

export type SearchHub = {
  type: string;
  title: string;
  items: MediaItem[];
};

type Tag = { tag: string };
type RoleTag = { tag: string; role?: string; thumb?: string };
type MarkerNode = {
  type?: string;
  startTimeOffset?: number;
  endTimeOffset?: number;
};
// Plex's per-track stream descriptor. `streamType` is 1=video, 2=audio,
// 3=subtitle. The same node shape is reused across all three.
type StreamNode = {
  id?: number | string;
  streamType?: number | string;
  language?: string;
  languageCode?: string;
  languageTag?: string;
  codec?: string;
  displayTitle?: string;
  extendedDisplayTitle?: string;
  forced?: boolean | number | string;
  default?: boolean | number | string;
  selected?: boolean | number | string;
};
type PartNode = { id?: number | string; Stream?: StreamNode[] };
type MediaNode = { Part?: PartNode[] };
export type MetadataNode = {
  ratingKey?: string | number;
  key?: string;
  type?: string;
  title?: string;
  summary?: string;
  thumb?: string;
  art?: string;
  year?: number;
  duration?: number;
  viewOffset?: number;
  contentRating?: string;
  rating?: number;
  Genre?: Tag[];
  Director?: Tag[];
  Writer?: Tag[];
  Role?: RoleTag[];
  Marker?: MarkerNode[];
  Media?: MediaNode[];
  parentTitle?: string;
  parentRatingKey?: string | number;
  grandparentTitle?: string;
  grandparentRatingKey?: string | number;
  index?: number;
  childCount?: number;
  leafCount?: number;
  librarySectionID?: string | number;
};

// Plex serializes booleans inconsistently — sometimes true, sometimes "1",
// sometimes 1. Normalize them all to a real boolean for the UI.
function asBool(v: unknown): boolean {
  return v === true || v === 1 || v === "1";
}

function mapStream(s: StreamNode): MediaStream {
  return {
    id: Number(s.id ?? 0),
    language: s.language,
    // Plex sometimes uses `languageCode` (older) and `languageTag` (newer).
    // Either is fine for matching against user prefs since we lowercase both.
    languageCode: s.languageCode ?? s.languageTag,
    codec: s.codec,
    // Prefer the extended title ("English (SDH)" vs. "English") since the
    // suffix distinguishes forced/SDH variants from the regular track.
    displayTitle: s.extendedDisplayTitle ?? s.displayTitle,
    forced: asBool(s.forced),
    default: asBool(s.default),
    selected: asBool(s.selected),
  };
}

function extractStreams(
  node: MetadataNode,
  streamType: 2 | 3,
): MediaStream[] | undefined {
  // We only look at the first Media/Part since Plex's transcoder also
  // addresses them by mediaIndex/partIndex 0 — picking from a different part
  // would need wider plumbing.
  const part = node.Media?.[0]?.Part?.[0];
  const streams = part?.Stream;
  if (!Array.isArray(streams)) return undefined;
  const filtered = streams.filter((s) => Number(s.streamType) === streamType);
  if (filtered.length === 0) return undefined;
  return filtered.map(mapStream);
}

export function mapItem(d: MetadataNode): MediaItem {
  return {
    ratingKey: String(d.ratingKey ?? ""),
    key: String(d.key ?? ""),
    type: String(d.type ?? ""),
    title: String(d.title ?? ""),
    summary: d.summary,
    thumb: d.thumb,
    art: d.art,
    year: d.year,
    duration: d.duration,
    viewOffset: d.viewOffset,
    contentRating: d.contentRating,
    rating: d.rating,
    genres: Array.isArray(d.Genre) ? d.Genre.map((g) => g.tag) : undefined,
    directors: Array.isArray(d.Director)
      ? d.Director.map((x) => x.tag)
      : undefined,
    writers: Array.isArray(d.Writer) ? d.Writer.map((x) => x.tag) : undefined,
    cast: Array.isArray(d.Role)
      ? d.Role.map((r) => ({ name: r.tag, role: r.role }))
      : undefined,
    parentTitle: d.parentTitle,
    parentRatingKey:
      d.parentRatingKey !== undefined ? String(d.parentRatingKey) : undefined,
    grandparentTitle: d.grandparentTitle,
    grandparentRatingKey:
      d.grandparentRatingKey !== undefined
        ? String(d.grandparentRatingKey)
        : undefined,
    index: d.index,
    childCount: d.childCount,
    leafCount: d.leafCount,
    librarySectionID:
      d.librarySectionID !== undefined
        ? String(d.librarySectionID)
        : undefined,
    markers: Array.isArray(d.Marker)
      ? d.Marker.filter(
          (m): m is Required<Pick<MarkerNode, "type" | "startTimeOffset" | "endTimeOffset">> =>
            typeof m.type === "string" &&
            typeof m.startTimeOffset === "number" &&
            typeof m.endTimeOffset === "number",
        ).map((m) => ({
          type: m.type,
          startMs: m.startTimeOffset,
          endMs: m.endTimeOffset,
        }))
      : undefined,
    audioStreams: extractStreams(d, 2),
    subtitleStreams: extractStreams(d, 3),
    partId: (() => {
      const raw = d.Media?.[0]?.Part?.[0]?.id;
      if (raw === undefined) return undefined;
      const n = Number(raw);
      return Number.isFinite(n) ? n : undefined;
    })(),
  };
}

export function displayTitle(item: MediaItem): string {
  if (item.type === "episode" && item.grandparentTitle) {
    return item.grandparentTitle;
  }
  if (item.type === "season" && item.parentTitle) {
    return item.parentTitle;
  }
  return item.title;
}

export function formatRuntime(durationMs: number | undefined): string {
  if (!durationMs) return "";
  const totalMinutes = Math.round(durationMs / 60000);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours > 0 && minutes > 0) return `${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h`;
  return `${minutes}m`;
}
