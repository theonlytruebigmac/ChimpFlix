import { NextRequest, NextResponse } from "next/server";
import { createPin, plexLinkUrl, plexOAuthUrl } from "@/lib/plex";
import { originFromRequest } from "@/lib/request-origin";
import { writePendingPin } from "@/lib/session";

export async function POST(req: NextRequest) {
  // strong=true gives us a 24-char opaque PIN that fits Plex's OAuth-style
  // redirect flow.
  const pin = await createPin({ strong: true });
  await writePendingPin(pin.id);

  // Resolve the origin Plex should redirect back to. Always derive it
  // server-side from trusted inputs (APP_PUBLIC_ORIGIN env first, then
  // the Host header). We deliberately do NOT accept a body-supplied
  // origin: a malicious caller could otherwise POST `origin=evil.com`
  // and have plex.tv bounce the victim to a fake app after a real
  // sign-in.
  const origin = originFromRequest(req);
  const forwardUrl = `${origin}/login?from_plex=1`;

  return NextResponse.json({
    pinId: pin.id,
    code: pin.code,
    linkUrl: plexLinkUrl(pin.code),
    authUrl: plexOAuthUrl(pin.code, forwardUrl),
    expiresAt: pin.expiresAt,
  });
}
