import { AdminScrollReset } from "@/components/admin/AdminScrollReset";
import { requireOwner } from "@/lib/chimpflix-server";

/// Admin (Server context) shell. The unified SettingsShell in the outer
/// /settings layout now owns the sidebar nav for BOTH contexts, so this
/// layout's only jobs are the server-side owner gate and resetting scroll
/// position on sub-route changes. Non-owners are bounced to /settings.
export default async function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  await requireOwner("/settings/admin");

  return (
    <>
      <AdminScrollReset />
      {children}
    </>
  );
}
