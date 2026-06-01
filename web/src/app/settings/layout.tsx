import { TopNav } from "@/components/TopNav";
import { SettingsShell } from "@/components/SettingsShell";
import { requireUser } from "@/lib/chimpflix-server";

/// Unified shell for everything under /settings. A single left sidebar
/// (SettingsShell) switches between the personal ("You") and server
/// ("Server", owner-only) contexts and hosts the ⌘K command palette;
/// the page renders in the content column to its right. The breadcrumb +
/// active sidebar item identify the page, so there's no page-title header.
export default async function SettingsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const user = await requireUser("/settings");

  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="mx-auto max-w-330 px-4 pb-24 pt-24 sm:px-6 sm:pt-28">
        <div className="flex flex-col gap-6 lg:flex-row lg:gap-8">
          <div className="lg:w-60 lg:shrink-0">
            <SettingsShell isOwner={user.role === "owner"} />
          </div>
          <div className="min-w-0 flex-1">{children}</div>
        </div>
      </div>
    </main>
  );
}
