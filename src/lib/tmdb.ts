import { env } from "./env";

const BASE = "https://api.themoviedb.org/3";

type SearchResult = {
  id: number;
  title?: string;
  name?: string;
  release_date?: string;
  first_air_date?: string;
};

type Video = {
  key: string;
  site: string;
  type: string;
  official?: boolean;
};

async function tmdb<T>(path: string): Promise<T | null> {
  const token = env.tmdbReadToken();
  if (!token) return null;
  try {
    const res = await fetch(`${BASE}${path}`, {
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/json",
      },
      // TMDB metadata is stable enough to cache aggressively.
      next: { revalidate: 60 * 60 * 24 },
    });
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  }
}

/**
 * Returns a YouTube video ID for the best available trailer of the given
 * title, or null if TMDB has nothing usable. Prefers official trailers.
 */
export async function findTrailerVideoId(
  type: "movie" | "tv",
  title: string,
  year?: number,
): Promise<string | null> {
  const q = encodeURIComponent(title);
  const yearParam =
    type === "movie"
      ? year
        ? `&primary_release_year=${year}`
        : ""
      : year
        ? `&first_air_date_year=${year}`
        : "";

  const search = await tmdb<{ results: SearchResult[] }>(
    `/search/${type}?query=${q}${yearParam}&include_adult=true`,
  );
  if (!search || search.results.length === 0) return null;
  const id = search.results[0].id;

  const videos = await tmdb<{ results: Video[] }>(
    `/${type}/${id}/videos`,
  );
  if (!videos || videos.results.length === 0) return null;

  const youtube = videos.results.filter((v) => v.site === "YouTube");
  const trailer =
    youtube.find((v) => v.type === "Trailer" && v.official) ??
    youtube.find((v) => v.type === "Trailer") ??
    youtube.find((v) => v.type === "Teaser") ??
    null;
  return trailer?.key ?? null;
}
