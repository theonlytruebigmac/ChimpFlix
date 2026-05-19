import { ImageResponse } from "next/og";
import { brandNameUpper } from "@/lib/env";

export const size = { width: 192, height: 192 };
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
          fontSize: 140,
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
