import Link from "next/link";
import { redirect } from "next/navigation";
import { brandName } from "@/lib/env";
import { getHomeUsers, type HomeUser } from "@/lib/plex";
import { readAdminToken } from "@/lib/session";
import { ProfilePicker } from "@/components/ProfilePicker";

export default async function SwitchProfilePage() {
  const adminToken = await readAdminToken();
  if (!adminToken) redirect("/login");

  let users: HomeUser[] = [];
  let errorMessage: string | null = null;
  try {
    users = await getHomeUsers(adminToken);
  } catch (e) {
    errorMessage = e instanceof Error ? e.message : String(e);
  }

  return (
    <main className="relative min-h-screen bg-black text-white">
      {users.length > 0 ? (
        <ProfilePicker users={users} />
      ) : (
        <div className="flex min-h-screen flex-col items-center justify-center px-6">
          <h1 className="mb-4 text-3xl font-medium tracking-tight">
            No profiles found
          </h1>
          <p className="mb-2 max-w-lg text-center text-sm text-white/65">
            Plex&apos;s home/users endpoint didn&apos;t return any profiles for
            this account. If you&apos;ve set up Plex Home with managed users,
            they should show up here.
          </p>
          {errorMessage && (
            <pre className="mt-4 max-w-lg overflow-auto rounded border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/70">
              {errorMessage}
            </pre>
          )}
          <Link
            href="/"
            className="mt-8 text-sm text-white underline underline-offset-4 hover:text-(--color-accent)"
          >
            Continue to {brandName()}
          </Link>
        </div>
      )}
    </main>
  );
}
