import type { Metadata } from "next";
import { Inter } from "next/font/google";
import { ColdStartOverlay } from "@/components/ColdStartOverlay";
import { NavPrefetch } from "@/components/NavPrefetch";
import { ServiceWorker } from "@/components/ServiceWorker";
import { isWarmerReady } from "@/lib/cache-warmer";
import { brandName } from "@/lib/env";
import { readToken } from "@/lib/session";
import "./globals.css";

const inter = Inter({
  variable: "--font-sans",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: brandName(),
  description: "Personal streaming library",
};

export default async function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  // Only consider showing the cold-start overlay for authenticated users.
  // Unauthenticated visitors (e.g., /login while the warmer hasn't fired
  // its first tick yet at boot) should see the normal page, not a
  // "Preparing your library…" screen that doesn't apply to them.
  const signedIn = (await readToken()) !== null;
  const initiallyReady = !signedIn || isWarmerReady();
  return (
    <html lang="en" className={`${inter.variable} h-full antialiased`}>
      <head>
        {/*
          Preconnect to the third-party origins the browser will need shortly
          after first paint:
            - plex.tv: serves the user-avatar images shown in the nav.
            - www.youtube.com / i.ytimg.com: trailer iframe + its thumbnail
              host. By the time the modal opens or the hero swaps to a
              trailer, the TLS handshake is already done.
          dns-prefetch is the broader fallback for browsers that ignore
          preconnect.
        */}
        <link rel="preconnect" href="https://plex.tv" crossOrigin="anonymous" />
        <link rel="dns-prefetch" href="https://plex.tv" />
        <link
          rel="preconnect"
          href="https://www.youtube.com"
          crossOrigin="anonymous"
        />
        <link rel="dns-prefetch" href="https://www.youtube.com" />
        <link
          rel="preconnect"
          href="https://i.ytimg.com"
          crossOrigin="anonymous"
        />
        <link rel="dns-prefetch" href="https://i.ytimg.com" />
      </head>
      <body className="min-h-full bg-black text-white font-sans">
        <ServiceWorker />
        <NavPrefetch />
        {/*
          Read the warmer state at SSR so the very first paint after a
          container restart is the "Preparing your library…" overlay
          rather than a screen full of skeletons. Once the warmer's
          first tick completes, the polling inside ColdStartOverlay
          reloads the page and the SSR check returns true on subsequent
          renders.
        */}
        <ColdStartOverlay initiallyReady={initiallyReady} />
        {children}
      </body>
    </html>
  );
}
