import { AdminNav } from "@/components/admin/AdminNav";
import { requireOwner } from "@/lib/chimpflix-server";

/// Admin shell. The outer /settings layout already provides TopNav and
/// the top tab strip; this layout adds the per-section sidebar nav for
/// the admin tabs and gates access to owners.
export default async function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  await requireOwner("/settings/admin");

  return (
    <div className="flex gap-8 pt-2">
      <aside className="sticky top-32 hidden h-fit w-56 shrink-0 lg:block">
        <AdminNav />
      </aside>
      <main className="min-w-0 flex-1">{children}</main>
    </div>
  );
}
