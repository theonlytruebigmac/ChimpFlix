// Adapt Rust API shapes to the Plex-shaped `MediaItem` the existing
// Hero/Rail/Card components consume. Lets us reuse the polished UI
// without rewriting components.
//
// `ratingKey` conventions (kept compatible with the modal and watch routes):
//   - "<id>"     — item (movie or show), passed through Number.parseInt.
//   - "e<id>"    — episode, the watch page strips the prefix.
//   - "s<id>"    — season; not currently routed independently, only
//                  surfaced inside a show's modal.

import type {
  Episode,
  Item,
  ItemDetail,
  ListedItem,
  OnDeckEntry,
  SeasonSummary,
} from "./chimpflix-api";
import type { MediaItem } from "./chimpflix-types";

export function adaptItem(item: Item | ListedItem): MediaItem {
  const media: MediaItem = {
    ratingKey: String(item.id),
    key: `/items/${item.id}`,
    type: item.kind,
    title: item.title,
    summary: item.summary ?? undefined,
    thumb: item.poster_path ?? undefined,
    art: item.backdrop_path ?? undefined,
    logo: item.logo_path ?? undefined,
    year: item.year ?? undefined,
    duration: item.duration_ms ?? undefined,
    rating: item.rating_audience ?? undefined,
    addedAt: item.added_at,
  };
  // ListedItem flattens play_state into the same object. When present,
  // expose it as viewOffset so Card's progress bar renders.
  const ps = (item as ListedItem).play_state;
  if (ps && ps.position_ms > 0) {
    media.viewOffset = ps.position_ms;
  }
  return media;
}

export function adaptItemDetail(detail: ItemDetail): MediaItem {
  const base = adaptItem(detail);
  base.genres = detail.genres;
  if (detail.play_state && detail.play_state.position_ms > 0) {
    base.viewOffset = detail.play_state.position_ms;
  }
  if (detail.kind === "show") {
    base.childCount = detail.seasons.length;
    base.leafCount = detail.seasons.reduce((n, s) => n + s.episode_count, 0);
  }
  return base;
}

export function adaptSeason(s: SeasonSummary, show: Item): MediaItem {
  return {
    ratingKey: `s${s.id}`,
    key: `/seasons/${s.id}`,
    type: "season",
    title: s.title ?? `Season ${s.season_number}`,
    thumb: show.poster_path ?? undefined,
    art: show.backdrop_path ?? undefined,
    parentTitle: show.title,
    parentRatingKey: String(show.id),
    index: s.season_number,
    leafCount: s.episode_count,
  };
}

export function adaptEpisode(ep: Episode, show: Item): MediaItem {
  return {
    ratingKey: `e${ep.id}`,
    key: `/episodes/${ep.id}`,
    type: "episode",
    title: ep.title,
    summary: ep.summary ?? undefined,
    thumb: ep.thumb_path ?? show.poster_path ?? undefined,
    art: show.backdrop_path ?? undefined,
    duration: ep.duration_ms ?? undefined,
    parentTitle: `Season ${ep.season_number}`,
    grandparentTitle: show.title,
    grandparentRatingKey: String(show.id),
    index: ep.episode_number,
  };
}

export function adaptOnDeck(entry: OnDeckEntry): MediaItem {
  if (entry.kind === "movie") {
    const m = adaptItem(entry.item);
    if (entry.play_state.position_ms > 0) {
      m.viewOffset = entry.play_state.position_ms;
    }
    return m;
  }
  const m = adaptEpisode(entry.episode, entry.show);
  if (entry.play_state.position_ms > 0) {
    m.viewOffset = entry.play_state.position_ms;
  }
  return m;
}
