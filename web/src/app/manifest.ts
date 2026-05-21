import type { MetadataRoute } from "next";
import { brandName } from "@/lib/env";

// Without a real manifest, mobile Chrome's "Add to Home Screen" creates
// an isolated shortcut whose cookie jar doesn't survive between launches
// — that's why mobile users were signing in every visit. Shipping a
// manifest with start_url + display=standalone turns the icon into a
// proper PWA that shares cookies + storage with the regular Chrome jar.
export default function manifest(): MetadataRoute.Manifest {
  const name = brandName();
  return {
    name,
    short_name: name,
    description: "Personal streaming library",
    start_url: "/",
    scope: "/",
    display: "standalone",
    orientation: "any",
    background_color: "#0d0d0d",
    theme_color: "#0d0d0d",
    icons: [
      {
        src: "/icon",
        sizes: "192x192",
        type: "image/png",
        purpose: "any",
      },
      {
        src: "/icon-512",
        sizes: "512x512",
        type: "image/png",
        purpose: "any",
      },
      {
        src: "/icon-512",
        sizes: "512x512",
        type: "image/png",
        purpose: "maskable",
      },
      {
        src: "/apple-icon",
        sizes: "180x180",
        type: "image/png",
      },
    ],
  };
}
