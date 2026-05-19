import { AdminMobileNav } from "@/components/admin/AdminMobileNav";
import { AdminNav } from "@/components/admin/AdminNav";
import { requireOwner } from "@/lib/chimpflix-server";

/// Admin shell. The outer /settings layout already provides TopNav and
/// the top tab strip; this layout adds the per-section sidebar nav for
/// the admin tabs and gates access to owners.
///
/// Responsive layout:
///   * Below md (≤768px): no sidebar, a hamburger trigger above the
///     content opens the AdminMobileNav drawer.
///   * md+ (≥768px): persistent sticky sidebar on the left.
export default async function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  await requireOwner("/settings/admin");

  return (
    <div className="flex flex-col gap-4 pt-2 md:flex-row md:gap-8">
      <aside className="sticky top-32 hidden h-fit w-56 shrink-0 md:block">
        <AdminNav />
      </aside>
      <main className="min-w-0 flex-1">
        <AdminMobileNav />
        {children}
      </main>
    </div>
  );
}
