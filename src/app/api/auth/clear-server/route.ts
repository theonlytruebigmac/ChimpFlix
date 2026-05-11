import { NextResponse } from "next/server";
import { originFromRequest } from "@/lib/request-origin";
import { clearActiveServer, readAdminToken } from "@/lib/session";

// Clears the active server cookie. Two callers:
//   - JSON: ProfileMenu's "Switch server" button (fetch + client redirect)
//   - Form POST (no JS): the "Pick a different server" button on
//     ServerUnreachable, which expects a redirect response so the
//     browser navigates after the form submits
async function handle(req: Request) {
  const masterToken = await readAdminToken();
  if (!masterToken) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }
  await clearActiveServer();

  const accept = req.headers.get("accept") ?? "";
  if (!accept.includes("application/json")) {
    // Use the browser's actual origin, not req.url (which reflects the
    // bind address — e.g. 0.0.0.0 in Docker — and would 303 the user
    // somewhere they can't reach).
    // `?manual=1` tells the page to skip auto-pick — the user just
    // failed to reach the previously-chosen connection, don't redo the
    // exact same pick.
    return NextResponse.redirect(
      `${originFromRequest(req)}/select-server?manual=1`,
      303,
    );
  }
  return NextResponse.json({ ok: true });
}

export const POST = handle;
