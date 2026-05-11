import { NextRequest, NextResponse } from "next/server";
import { persistBootstrapAuth } from "@/lib/admin-token-store";
import { ensureWarmerStarted } from "@/lib/cache-warmer";
import {
  assertProbeUriResolvesSafely,
  getResources,
  pickReachableConnectionUri,
  type ServerAuth,
} from "@/lib/plex";
import {
  readAdminToken,
  readToken,
  writeActiveServer,
} from "@/lib/session";

// Sets the active Plex server for this session. The client POSTs the
// chosen server's `id`; we re-fetch resources (or use the cached list),
// validate the user actually has access to that server, then write the
// cf_server cookie with the server URL + per-server access token.
//
// Validating server-side prevents a malicious client from setting
// arbitrary URLs into the cookie.
export async function POST(req: NextRequest) {
  const masterToken = await readAdminToken();
  if (!masterToken) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }

  let body: { serverId?: string; preferUri?: string } = {};
  try {
    body = await req.json();
  } catch {
    // ignore — validated below
  }
  if (!body.serverId) {
    return NextResponse.json(
      { error: "serverId is required" },
      { status: 400 },
    );
  }

  const resources = await getResources(masterToken).catch(() => null);
  if (!resources) {
    return NextResponse.json(
      { error: "could not list Plex servers" },
      { status: 502 },
    );
  }

  const target = resources.find((r) => r.id === body.serverId);
  if (!target) {
    return NextResponse.json(
      { error: "server not found in your account's resources" },
      { status: 404 },
    );
  }

  // Use the connection the client preferred when set + present, otherwise
  // probe each candidate and pick the first that actually answers. This
  // matters for setups where the server reports a "local" address that's
  // really a Docker bridge IP (or any other non-routable LAN), which the
  // app host can't reach — in that case we fall through to the public
  // plex.direct URL or the relay.
  const explicit = body.preferUri
    ? target.connections.find((c) => c.uri === body.preferUri)?.uri
    : null;
  const url = explicit ?? (await pickReachableConnectionUri(target));
  if (!url) {
    return NextResponse.json(
      {
        error:
          "couldn't reach this Plex server on any of its advertised addresses",
      },
      { status: 502 },
    );
  }
  // Before committing the URL to the session cookie, verify the
  // hostname doesn't resolve to a blocked address (loopback / metadata
  // / link-local). Defends against a compromised plex.tv response
  // pointing us at a hostname whose A record is 127.0.0.1.
  try {
    await assertProbeUriResolvesSafely(url);
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : "rejected connection URI" },
      { status: 400 },
    );
  }

  const auth: ServerAuth = {
    id: target.id,
    url,
    accessToken: target.accessToken,
  };

  await writeActiveServer(auth);

  // Update the boot-time bootstrap so a container restart immediately
  // resumes warming this server. Only do this when the active session
  // matches the master token — i.e., the user is signed in directly,
  // not switched into a managed user.
  const activeSessionToken = await readToken();
  if (activeSessionToken === masterToken) {
    await persistBootstrapAuth(auth);
  }

  // Kick the warmer off immediately so the first nav lands on cached
  // data instead of paying the first cycle's full cost.
  ensureWarmerStarted(auth);

  return NextResponse.json({ ok: true, server: { id: auth.id, url } });
}
