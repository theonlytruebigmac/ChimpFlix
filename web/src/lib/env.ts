// User-visible brand name. Pulled from NEXT_PUBLIC_BRAND_NAME so it's
// inlined into client bundles at build time and can be read from both
// server and client components. Falls back to "ChimpFlix" when unset.
//
// NEXT_PUBLIC_* values are baked at `next build` time, not read at
// runtime — changing the env var requires a rebuild. In docker, that
// means passing it through `build.args` in docker-compose.yml.
export function brandName(): string {
  return process.env.NEXT_PUBLIC_BRAND_NAME?.trim() || "ChimpFlix";
}

// Uppercase styling for hero/logo treatments. NEXT_PUBLIC_BRAND_NAME_UPPER
// is an explicit override; otherwise we auto-uppercase brandName().
export function brandNameUpper(): string {
  const override = process.env.NEXT_PUBLIC_BRAND_NAME_UPPER?.trim();
  return override || brandName().toUpperCase();
}
