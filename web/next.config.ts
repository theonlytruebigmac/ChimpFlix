import type { NextConfig } from "next";

const apiTarget = process.env.CHIMPFLIX_API_URL ?? "http://localhost:8080";

const nextConfig: NextConfig = {
  output: "standalone",
  allowedDevOrigins: ["172.16.103.228"],
  async rewrites() {
    // Proxy /api/v1/* to the Rust backend so the browser sees the whole
    // app on a single origin. The legacy /api/auth/* Plex routes are
    // Next.js route handlers in src/app/api and are left untouched.
    return [
      {
        source: "/api/v1/:path*",
        destination: `${apiTarget}/api/v1/:path*`,
      },
    ];
  },
};

export default nextConfig;
