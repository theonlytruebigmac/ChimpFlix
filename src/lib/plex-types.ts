export type CastMember = { name: string; role?: string };

export type Marker = {
  type: string; // "intro", "credits", "commercial"
  startMs: number;
  endMs: number;
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
  markers?: Marker[];
  librarySectionID?: string;
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
  parentTitle?: string;
  parentRatingKey?: string | number;
  grandparentTitle?: string;
  grandparentRatingKey?: string | number;
  index?: number;
  childCount?: number;
  leafCount?: number;
  librarySectionID?: string | number;
};

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
