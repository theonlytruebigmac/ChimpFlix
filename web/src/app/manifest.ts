import type { MetadataRoute } from "next";
import { brandName } from "@/lib/env";

// Without a real manifest, mobile Chrome's "Add to Home Screen" creates
// an isolated shortcut whose cookie jar doesn't survive between launches
// — that's why mobile users were signing in every visit. Shipping a
// manifest with start_url + display=standalone turns the icon into a
// proper PWA that shares cookies + storage with the regular Chrome jar.
export default function manifest(): MetadataRoute.Manifest {
  const name = brandName();
  const base: MetadataRoute.Manifest = {
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
        src: "/icons/icon-192.png",
        sizes: "192x192",
        type: "image/png",
        purpose: "any",
      },
      {
        src: "/icons/icon-512.png",
        sizes: "512x512",
        type: "image/png",
        purpose: "any",
      },
      // Adaptive-launcher safe-zone variants. Source content is scaled
      // to ~80% on a solid dark square so Pixel / Samsung / Android
      // circle/squircle/teardrop crops keep the wordmark legible.
      {
        src: "/icons/icon-192-maskable.png",
        sizes: "192x192",
        type: "image/png",
        purpose: "maskable",
      },
      {
        src: "/icons/icon-512-maskable.png",
        sizes: "512x512",
        type: "image/png",
        purpose: "maskable",
      },
      {
        src: "/icons/apple-touch-icon.png",
        sizes: "180x180",
        type: "image/png",
      },
    ],
  };
  return {
    ...base,
    // A stable `id` makes the OS treat this as one installable app, and
    // `launch_handler` with `focus-existing` refocuses the single running
    // app window when the media / "now playing" notification is tapped —
    // instead of Chrome spawning a fresh (blank) tab, which is the symptom
    // users hit tapping the Android cast notification. `navigate-existing`
    // is the fallback Chrome applies when focus-existing isn't honored.
    // (Cast/now-playing refocus is materially more reliable from an
    // installed PWA than a bare browser tab, which Chrome can't
    // single-instance.) Asserted because `launch_handler` is newer than
    // Next's MetadataRoute.Manifest type.
    id: "/",
    launch_handler: { client_mode: ["focus-existing", "navigate-existing"] },
  } as MetadataRoute.Manifest;
}
