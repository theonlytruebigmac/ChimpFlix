import type { Metadata, Viewport } from "next";
import { Inter } from "next/font/google";
import { NavPrefetch } from "@/components/NavPrefetch";
import { ServiceWorker } from "@/components/ServiceWorker";
import { brandName } from "@/lib/env";
import "./globals.css";

const inter = Inter({
  variable: "--font-sans",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: brandName(),
  description: "Personal streaming library",
  // iOS Safari ignores the web app manifest; these meta tags are the
  // only way to opt iOS "Add to Home Screen" into standalone mode.
  appleWebApp: {
    capable: true,
    title: brandName(),
    statusBarStyle: "black-translucent",
  },
};

export const viewport: Viewport = {
  themeColor: "#0d0d0d",
  width: "device-width",
  initialScale: 1,
  // `cover` lets the layout extend under notches / dynamic islands.
  // The page background is solid `--color-background` so the area
  // around the safe-area inset still reads as part of the app rather
  // than as a black bar.
  viewportFit: "cover",
};

export default async function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" className={`${inter.variable} h-full antialiased`}>
      <head>
        {/*
          The legacy Plex/YouTube preconnects are gone now. We still
          preconnect to TMDB's image CDN since every browse page paints
          posters from there.
        */}
        <link
          rel="preconnect"
          href="https://image.tmdb.org"
          crossOrigin="anonymous"
        />
        <link rel="dns-prefetch" href="https://image.tmdb.org" />
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
      <body className="min-h-full bg-background text-white font-sans">
        <ServiceWorker />
        <NavPrefetch />
        {children}
      </body>
    </html>
  );
}
