import { NextResponse } from "next/server";
import { clearBootstrapAuth } from "@/lib/admin-token-store";
import { originFromRequest } from "@/lib/request-origin";
import {
  clearActiveProfile,
  clearActiveServer,
  clearAdminToken,
  clearPendingPin,
  clearToken,
} from "@/lib/session";

async function handle(req: Request) {
  await clearToken();
  await clearAdminToken();
  await clearActiveProfile();
  await clearActiveServer();
  await clearPendingPin();
  // Drop the on-disk bootstrap so a future container restart doesn't try
  // to warm the cache with an auth the user has explicitly revoked.
  await clearBootstrapAuth();

  // Form POSTs (sign-out buttons in error pages, ProfileMenu) get a
  // redirect to /login so the browser navigates without needing client
  // JS. Use the browser's origin (not req.url, which can be the bind
  // address) so the redirect target is reachable.
  const accept = req.headers.get("accept") ?? "";
  if (!accept.includes("application/json")) {
    return NextResponse.redirect(`${originFromRequest(req)}/login`, 303);
  }
  return NextResponse.json({ status: "ok" });
}

export const POST = handle;
