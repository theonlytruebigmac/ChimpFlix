import type { Metadata } from "next";
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
