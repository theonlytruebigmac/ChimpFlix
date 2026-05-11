function required(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`Missing required env var: ${name}`);
  return v;
}

// User-visible brand name. Pulled from NEXT_PUBLIC_BRAND_NAME so it's
// inlined into client bundles at build time and can be read from both
// server and client components. Falls back to "ChimpFlix" when unset.
//
// Used everywhere a wordmark appears in the UI: top nav, login,
// modals, page titles, "New on …" rails.
export function brandName(): string {
  return process.env.NEXT_PUBLIC_BRAND_NAME?.trim() || "ChimpFlix";
}

// Uppercase styling for hero/logo treatments that historically rendered
// the brand in all caps. Kept as a helper so a future deployment can
// override it (NEXT_PUBLIC_BRAND_NAME_UPPER) without losing the
// auto-uppercased default.
export function brandNameUpper(): string {
  const override = process.env.NEXT_PUBLIC_BRAND_NAME_UPPER?.trim();
  return override || brandName().toUpperCase();
}

export const env = {
  // Optional in multi-tenant mode. When set, it's used as a fallback /
  // default if the active session doesn't have a server selection cookie
  // (e.g. legacy single-server deployments). Returns null if unset or
  // empty so callers can branch on it.
  plexServerUrlDefault: () => {
    const v = process.env.PLEX_SERVER_URL?.trim();
    return v ? v.replace(/\/+$/, "") : null;
  },
  plexClientIdentifier: () => required("PLEX_CLIENT_IDENTIFIER"),
  plexProductName: () => process.env.PLEX_PRODUCT_NAME ?? brandName(),
  plexDeviceName: () => process.env.PLEX_DEVICE_NAME ?? brandName(),
  sessionSecret: () => required("SESSION_SECRET"),
  tmdbApiKey: () => process.env.TMDB_API_KEY ?? "",
  tmdbReadToken: () => process.env.TMDB_READ_TOKEN ?? "",
  // Canonical public origin (scheme + host + optional port) used for
  // redirect targets and the Secure cookie decision. Set this in any
  // production deployment so the app doesn't trust attacker-supplied
  // Host / X-Forwarded-Proto headers. Returns null when unset.
  //
  // Example: APP_PUBLIC_ORIGIN=https://chimpflix.example.com
  publicOrigin: () => {
    const v = process.env.APP_PUBLIC_ORIGIN?.trim();
    return v ? v.replace(/\/+$/, "") : null;
  },
  // Opt-in flag for honoring X-Forwarded-* headers when no
  // APP_PUBLIC_ORIGIN is configured. Default is *not* to trust them, so
  // that a direct connection to Node can't spoof scheme/host. Set when
  // behind a TLS-terminating proxy.
  trustProxy: () => process.env.APP_TRUST_PROXY === "1",
};
