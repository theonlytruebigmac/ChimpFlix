// One Plex transcoded image URL at the requested logical pixel dimensions.
// `format=webp` cuts byte size 30-50% vs JPEG; the proxy ships an
// immutable Cache-Control header so a given URL never re-fetches.
export function plexImage(
  path: string | undefined,
  width: number,
  height: number,
): string | null {
  if (!path) return null;
  return buildUrl(path, width, height);
}

// Responsive set: gives the browser 1x and 2x variants and lets it pick
// based on device pixel ratio. Saves bytes on standard-DPR laptops while
// still hitting a sharp render on retina/4K displays. Pass the *logical*
// (CSS) dimensions; the helper computes the physical pixel sizes.
export function plexSrcSet(
  path: string | undefined,
  width: number,
  height: number,
): string | undefined {
  if (!path) return undefined;
  const variants = [1, 2] as const;
  return variants
    .map((scale) => {
      const w = Math.round(width * scale);
      const h = Math.round(height * scale);
      return `${buildUrl(path, w, h)} ${scale}x`;
    })
    .join(", ");
}

function buildUrl(path: string, width: number, height: number): string {
  // Absolute URLs (TMDB artwork from the Rust backend) are already sized
  // appropriately by the path itself (e.g. .../w500/abc.jpg). Pass them
  // through unchanged — the Plex photo transcoder isn't in the path now.
  if (/^https?:\/\//i.test(path)) return path;
  // Server-relative paths returned by the ChimpFlix backend (e.g.
  // /api/v1/items/123/poster/blob for user-uploaded artwork) are also
  // self-sized — let the browser pull them directly.
  if (path.startsWith("/api/")) return path;
  const url = encodeURIComponent(path);
  return `/api/plex/photo/:/transcode?width=${width}&height=${height}&minSize=1&upscale=1&format=webp&url=${url}`;
}
