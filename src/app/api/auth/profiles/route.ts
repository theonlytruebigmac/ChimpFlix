import { NextResponse } from "next/server";
import { getHomeUsers } from "@/lib/plex";
import { readAdminToken } from "@/lib/session";

export async function GET() {
  const adminToken = await readAdminToken();
  if (!adminToken) {
    return NextResponse.json({ users: [] }, { status: 401 });
  }
  try {
    const users = await getHomeUsers(adminToken);
    return NextResponse.json({ users });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e), users: [] },
      { status: 500 },
    );
  }
}
