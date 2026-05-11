import { NextResponse } from "next/server";
import { getUser } from "@/lib/plex";
import {
  readActiveProfile,
  readAdminToken,
  readToken,
} from "@/lib/session";

export async function GET() {
  const token = await readToken();
  if (!token) return NextResponse.json({ user: null }, { status: 401 });
  const adminToken = await readAdminToken();
  const isAdmin = adminToken !== null && token === adminToken;
  // hasAdmin: a stashed admin token exists alongside the active session.
  // True for the admin themselves, and also true for managed users since the
  // admin's token stays in the cf_admin cookie when they switch persona. The
  // menu uses this to decide whether to expose the "Switch profile" entry —
  // managed users still need a way back to admin without re-doing OAuth.
  const hasAdmin = adminToken !== null;

  // Fast path: if a managed-user switch stashed the profile identity, use
  // that. Avoids a plex.tv roundtrip on every page load and works for
  // managed-user tokens (which can't query /api/v2/user).
  const profile = await readActiveProfile();
  if (profile && !isAdmin) {
    return NextResponse.json({ user: profile, isAdmin, hasAdmin });
  }

  try {
    const user = await getUser(token);
    return NextResponse.json({ user, isAdmin, hasAdmin });
  } catch {
    // Last-ditch fallback to whatever we have cached so the avatar can still
    // render (otherwise the menu shows the "?" placeholder).
    if (profile)
      return NextResponse.json({ user: profile, isAdmin, hasAdmin });
    return NextResponse.json(
      { user: null, isAdmin: false, hasAdmin: false },
      { status: 401 },
    );
  }
}
