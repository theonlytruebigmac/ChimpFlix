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
  const url = encodeURIComponent(path);
  return `/api/plex/photo/:/transcode?width=${width}&height=${height}&minSize=1&upscale=1&format=webp&url=${url}`;
}
