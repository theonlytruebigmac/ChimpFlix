import type { NextConfig } from "next";

const apiTarget = process.env.CHIMPFLIX_API_URL ?? "http://localhost:8080";

const nextConfig: NextConfig = {
  output: "standalone",
  async rewrites() {
    // Proxy /api/v1/* to the Rust backend so the browser sees the whole
    // app on a single origin. Override the target host via
    // `CHIMPFLIX_API_URL` (compose sets it to http://server:8080).
    return [
      {
        source: "/api/v1/:path*",
        destination: `${apiTarget}/api/v1/:path*`,
      },
    ];
  },
};

export default nextConfig;
