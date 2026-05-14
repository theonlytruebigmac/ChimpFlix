import { NextRequest, NextResponse } from "next/server";
import {
  readHiddenLibraries,
  writeHiddenLibraries,
} from "@/lib/library-prefs";
import { readToken } from "@/lib/session";

export async function GET() {
  const token = await readToken();
  if (!token) {
    return NextResponse.json({ hidden: [] }, { status: 401 });
  }
  const set = await readHiddenLibraries();
  return NextResponse.json({ hidden: [...set] });
}

export async function POST(req: NextRequest) {
  const token = await readToken();
  if (!token) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }

  let body: { hidden?: unknown } = {};
  try {
    body = await req.json();
  } catch {
    // ignored; validation below
  }
  const incoming = body.hidden;
  if (!Array.isArray(incoming)) {
    return NextResponse.json(
      { error: "hidden must be an array of section keys" },
      { status: 400 },
    );
  }
  const cleaned = incoming.filter((x): x is string => typeof x === "string");
  await writeHiddenLibraries(cleaned);
  return NextResponse.json({ ok: true, hidden: cleaned });
}
