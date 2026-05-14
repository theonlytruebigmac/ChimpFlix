// Persisted session-bootstrap store. Lets the cache warmer prime the
// Plex data cache at server-process boot, before the first user request
// lands — without this, every container restart pays a "first hit cold"
// penalty because the warmer doesn't know which server to pre-fetch.
//
// In multi-tenant mode we persist a full ServerAuth (server id + URL +
// access token), not just a master token, because the warmer needs to
// know which Plex server to query.
//
// Trade-off: writes the user's per-server access token to a file on
// disk. For a single-user homelab deployment this is no worse than the
// cookie sitting in the user's browser. Path is .gitignored /
// .dockerignored so it doesn't leak into images or commits.

import { promises as fs } from "node:fs";
import path from "node:path";
import type { ServerAuth } from "./plex";

const FILENAME = ".app-admin-token";

function tokenPath(): string {
  // Honor APP_STATE_DIR for Docker deployments that mount a volume to
  // persist state across container restarts. Falls back to the working
  // dir for local dev / single-server homelab setups where the file
  // being ephemeral is fine.
  const dir = process.env.APP_STATE_DIR?.trim() || process.cwd();
  return path.join(dir, FILENAME);
}

export async function persistBootstrapAuth(auth: ServerAuth): Promise<void> {
  try {
    await fs.writeFile(tokenPath(), JSON.stringify(auth), { mode: 0o600 });
  } catch {
    // Persistence is an optimization, not a correctness requirement.
  }
}

export async function readBootstrapAuth(): Promise<ServerAuth | null> {
  try {
    const data = await fs.readFile(tokenPath(), "utf-8");
    const parsed = JSON.parse(data) as Partial<ServerAuth>;
    if (!parsed?.id || !parsed?.url || !parsed?.accessToken) return null;
    return {
      id: String(parsed.id),
      url: String(parsed.url),
      accessToken: String(parsed.accessToken),
    };
  } catch {
    return null;
  }
}

export async function clearBootstrapAuth(): Promise<void> {
  try {
    await fs.unlink(tokenPath());
  } catch {
    // already gone
  }
}
