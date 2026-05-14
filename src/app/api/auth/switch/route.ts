import { NextRequest, NextResponse } from "next/server";
import { getHomeUsers, switchHomeUser } from "@/lib/plex";
import {
  clearActiveProfile,
  readAdminToken,
  readToken,
  writeActiveProfile,
  writeToken,
} from "@/lib/session";

export async function POST(req: NextRequest) {
  const adminToken = await readAdminToken();
  if (!adminToken) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }

  let body: { userId?: number; pin?: string } = {};
  try {
    body = await req.json();
  } catch {
    // ignore — body validation below catches missing userId
  }

  if (typeof body.userId !== "number") {
    return NextResponse.json(
      { error: "userId is required" },
      { status: 400 },
    );
  }

  const activeToken = await readToken();
  const isCurrentlyAdmin = activeToken === adminToken;

  // Look up which userId the admin token belongs to so we can detect a
  // "managed → admin" swap. Cached upstream so this is cheap on repeat hits.
  let users;
  try {
    users = await getHomeUsers(adminToken);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return NextResponse.json({ error: msg }, { status: 502 });
  }
  const adminUser = users.find((u) => u.admin);
  const isReturnToAdmin =
    adminUser !== undefined && body.userId === adminUser.id;

  // Permission rule:
  //   - Admin: can switch to any profile (master account).
  //   - Managed user: can only switch back to admin. Lateral managed→managed
  //     jumps require the admin to do the switching, so a guest can't roam
  //     between accounts.
  if (!isCurrentlyAdmin && !isReturnToAdmin) {
    return NextResponse.json(
      {
        error:
          "only the master Plex account can switch to other profiles. " +
          "Switch back to the admin first.",
      },
      { status: 403 },
    );
  }

  // Managed → admin: shortcut. We already have the admin's authToken in the
  // cf_admin cookie, so there's nothing to ask plex.tv for. Just promote it
  // back to the active session and clear the stashed managed-user identity.
  if (isReturnToAdmin && !isCurrentlyAdmin) {
    await writeToken(adminToken);
    await clearActiveProfile();
    return NextResponse.json({ ok: true });
  }

  try {
    const newToken = await switchHomeUser(adminToken, body.userId, body.pin);
    await writeToken(newToken);
    // Stash the chosen profile's identity so the avatar/menu can render
    // immediately on subsequent pages without a plex.tv roundtrip (which
    // returns 401 for managed-user tokens anyway).
    if (body.userId === adminUser?.id) {
      // Admin → admin self-switch: drop any stale managed-user profile.
      await clearActiveProfile();
    } else {
      const picked = users.find((u) => u.id === body.userId);
      if (picked) {
        await writeActiveProfile({
          id: picked.id,
          username: picked.title || picked.username,
          email: picked.username,
          thumb: picked.thumb,
        });
      }
    }
    return NextResponse.json({ ok: true });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    // Plex returns 401 when the PIN is wrong; surface that as a clear status.
    const status = msg.includes("401") ? 401 : 400;
    return NextResponse.json({ error: msg }, { status });
  }
}
