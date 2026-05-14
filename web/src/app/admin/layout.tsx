import Link from "next/link";
import { AdminNav } from "@/components/admin/AdminNav";
import { TopNav } from "@/components/TopNav";
import { requireOwner } from "@/lib/chimpflix-server";

export default async function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Server-side owner gate. Non-owners are redirected to /settings.
  const user = await requireOwner("/admin");

  return (
    <div className="min-h-screen bg-background">
      <TopNav />
      <div className="mx-auto flex max-w-7xl gap-8 px-6 pb-24 pt-28">
        <aside className="sticky top-28 hidden h-fit w-56 shrink-0 flex-col gap-6 lg:flex">
          <div>
            <div className="text-xs uppercase tracking-wider text-white/40">
              Signed in as
            </div>
            <div className="mt-1 text-sm font-medium">
              {user.display_name ?? user.username}
              <span className="ml-2 rounded bg-red-500/20 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-red-300">
                Owner
              </span>
            </div>
            <Link
              href="/settings"
              className="mt-2 inline-block text-xs text-white/40 hover:text-white/60"
            >
              ← User settings
            </Link>
          </div>
          <AdminNav />
        </aside>

        <main className="min-w-0 flex-1">{children}</main>
      </div>
    </div>
  );
}
