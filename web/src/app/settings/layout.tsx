import { TopNav } from "@/components/TopNav";
import { SettingsTabs } from "@/components/SettingsTabs";
import { requireUser } from "@/lib/chimpflix-server";

/// Unified shell for everything under /settings: the per-user tabs
/// (Account, Player, Integrations, Libraries) plus the admin section
/// when the signed-in user is an owner. The Admin tab is just another
/// link on the same strip — the deeper admin sidebar nav is owned by
/// /settings/admin/layout.tsx.
export default async function SettingsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const user = await requireUser("/settings");

  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="mx-auto max-w-7xl px-4 pb-24 pt-24 sm:px-6 sm:pt-28">
        <header className="mb-6 flex flex-col items-start gap-2 sm:flex-row sm:items-baseline sm:justify-between sm:gap-4">
          <h1 className="text-3xl font-bold tracking-tight sm:text-4xl">Settings</h1>
          <div className="text-xs text-white/45">
            Signed in as{" "}
            <span className="text-white/80">
              {user.display_name ?? user.username}
            </span>
            {user.role === "owner" && (
              <span className="ml-2 rounded bg-red-500/20 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-red-300">
                Owner
              </span>
            )}
          </div>
        </header>
        <SettingsTabs isOwner={user.role === "owner"} />
        <div className="mt-6">{children}</div>
      </div>
    </main>
  );
}
