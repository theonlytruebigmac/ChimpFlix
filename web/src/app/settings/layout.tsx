import { TopNav } from "@/components/TopNav";
import { SettingsShell } from "@/components/SettingsShell";
import { requireUser } from "@/lib/chimpflix-server";
import "./console.css";

/// Unified shell for everything under /settings. A single left sidebar
/// (SettingsShell) switches between the personal ("You") and server
/// ("Server", owner-only) contexts and hosts the ⌘K command palette;
/// the page renders in the content column to its right. The breadcrumb +
/// active sidebar item identify the page, so there's no page-title header.
///
/// `.cf-console` carries the redesign design-system tokens + component
/// classes (console.css) down to both the sidebar and the page body so
/// every page renders in the mockup's visual language. Page content is
/// centred in a 1080px column (.cf-content-inner) per the mockup.
export default async function SettingsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const user = await requireUser("/settings");

  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="cf-console mx-auto max-w-350 px-4 pb-24 pt-24 sm:px-6 sm:pt-28">
        <div className="flex flex-col gap-6 lg:flex-row lg:gap-10">
          <div className="lg:w-64 lg:shrink-0">
            <SettingsShell isOwner={user.role === "owner"} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="cf-content-inner">{children}</div>
          </div>
        </div>
      </div>
    </main>
  );
}
