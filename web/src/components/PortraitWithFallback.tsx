"use client";

import { useState } from "react";

/// Portrait `<img>` (person headshot, profile photo) with an inline
/// initials fallback when the source is missing OR the URL 404s. The
/// fallback covers both the "no photo_url stored" case (server set
/// it to null) and the "photo_url exists but TMDB CDN returns 404"
/// case (which used to flash the broken-image icon for a frame
/// before settling). The render swaps to initials once the load
/// fails — no flash, no broken icon.
export function PortraitWithFallback({
  src,
  name,
  className,
}: {
  src: string | null;
  name: string;
  className?: string;
}) {
  const [failed, setFailed] = useState(false);

  if (!src || failed) {
    return (
      <div
        className={`flex items-center justify-center text-5xl font-bold text-white/30 ${className ?? ""}`}
      >
        {name
          .split(" ")
          .map((p) => p[0])
          .slice(0, 2)
          .join("")}
      </div>
    );
  }

  return (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={src}
      alt={name}
      className={className}
      onError={() => setFailed(true)}
    />
  );
}
