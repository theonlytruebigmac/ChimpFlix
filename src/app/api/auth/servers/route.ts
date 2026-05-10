import { NextResponse } from "next/server";
import {
  getResources,
  pickConnectionUri,
  type PlexResource,
} from "@/lib/plex";
import { readAdminToken } from "@/lib/session";

// Lists Plex servers the signed-in user can access. Used by the
// /select-server picker on first sign-in and by the Switch Server menu
// later.
//
// We return a simplified shape (id, name, owned, connection URI we'd
// pick by default, list of all candidates) so the client can render a
// picker without re-running the connection-priority logic.
export async function GET() {
  const masterToken = await readAdminToken();
  if (!masterToken) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }

  let resources: PlexResource[];
  try {
    resources = await getResources(masterToken);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return NextResponse.json({ error: msg }, { status: 502 });
  }

  // Dump the raw connection list per server. Useful when troubleshooting
  // "which addresses is my Plex actually advertising?" — common for
  // Docker / multi-NIC setups where the local IP isn't what you think.
  for (const r of resources) {
    // eslint-disable-next-line no-console
    console.log(
      `[plex] server "${r.name}" (owned=${r.owned}) connections:`,
      r.connections.map((c) => ({
        uri: c.uri,
        local: c.local,
        relay: c.relay,
        address: `${c.address}:${c.port}`,
      })),
    );
  }

  // Sort: owned servers first (most likely the user's primary), then
  // online (presence=true), then alphabetical.
  resources.sort((a, b) => {
    if (a.owned !== b.owned) return a.owned ? -1 : 1;
    if (a.presence !== b.presence) return a.presence ? -1 : 1;
    return a.name.localeCompare(b.name);
  });

  // Never include accessToken in the response — it's an authenticated
  // per-server credential that has no business in the browser. The
  // /api/auth/select-server route re-validates the chosen serverId
  // against the user's resources and writes the token straight into the
  // httpOnly cookie.
  const servers = resources.map((r) => ({
    id: r.id,
    name: r.name,
    owned: r.owned,
    presence: r.presence,
    productVersion: r.productVersion,
    platform: r.platform,
    defaultUri: pickConnectionUri(r),
    connections: r.connections.map((c) => ({
      uri: c.uri,
      local: c.local,
      relay: c.relay,
    })),
  }));

  return NextResponse.json({ servers });
}
