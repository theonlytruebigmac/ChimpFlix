import { ImageResponse } from "next/og";
import { brandNameUpper } from "@/lib/env";

// 512x512 variant for installable PWA splash screen + high-DPI launchers.
// Marked "any maskable" in manifest.ts so adaptive-icon launchers (Pixel,
// Samsung) can crop it without losing the brand letter.
export const size = { width: 512, height: 512 };
export const contentType = "image/png";

export default function Icon() {
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
          fontSize: 380,
          fontWeight: 900,
          letterSpacing: -16,
          fontFamily: "system-ui, sans-serif",
        }}
      >
        {letter}
      </div>
    ),
    { ...size },
  );
}
