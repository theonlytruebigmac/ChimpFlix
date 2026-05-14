import { NextResponse } from "next/server";
import { checkPin } from "@/lib/plex";
import {
  clearPendingPin,
  readPendingPin,
  writeAdminToken,
  writeToken,
} from "@/lib/session";

// POST (not GET) because this consumes the pending PIN cookie and
// writes session cookies — those are side-effects that don't belong on
// a GET. A side-effecting GET could be triggered by a third-party
// `<img>` tag during the OAuth window.
export async function POST() {
  const pinId = await readPendingPin();
  if (!pinId) {
    return NextResponse.json({ status: "no_pending_pin" }, { status: 400 });
  }
  const pin = await checkPin(pinId);
  if (!pin.authToken) {
    return NextResponse.json({ status: "pending" }, { status: 202 });
  }
  // Initial sign-in: the same Plex token serves as both the admin token (for
  // listing/switching home profiles + listing servers) and the active
  // session token (until the user picks a different profile).
  await writeAdminToken(pin.authToken);
  await writeToken(pin.authToken);
  await clearPendingPin();
  // We deliberately don't bootstrap the warmer or persist the bootstrap
  // file here — neither has a target server yet. Both happen in
  // /api/auth/select-server once the user picks a server, which is the
  // next step in the sign-in flow.
  return NextResponse.json({ status: "ok" });
}
