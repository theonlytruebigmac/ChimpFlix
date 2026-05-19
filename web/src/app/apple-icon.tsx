import { ImageResponse } from "next/og";
import { brandNameUpper } from "@/lib/env";

// iOS Safari does not honor web app manifest icons — it uses the
// `apple-touch-icon` link, which Next.js auto-generates from this file.
// Required for "Add to Home Screen" on iOS to show the right artwork.
export const size = { width: 180, height: 180 };
export const contentType = "image/png";

export default function AppleIcon() {
  const letter = brandNameUpper().charAt(0) || "C";
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          background: "#e50914",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: "white",
          fontSize: 130,
          fontWeight: 900,
          letterSpacing: -6,
          fontFamily: "system-ui, sans-serif",
        }}
      >
        {letter}
      </div>
    ),
    { ...size },
  );
}
