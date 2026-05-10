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

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
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
        {children}
      </body>
    </html>
  );
}
