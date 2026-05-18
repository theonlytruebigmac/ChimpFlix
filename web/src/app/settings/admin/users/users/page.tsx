import { requireOwner } from "@/lib/chimpflix-server";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { SettingsUsersClient } from "@/components/SettingsUsersClient";

export default async function AdminUsersPage() {
  const owner = await requireOwner("/settings/admin/users/users");
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Users"
        description="Owner and viewer accounts. Promote/demote and delete; you cannot modify your own role here."
      />
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsUsersClient currentUserId={owner.id} />
      </section>
    </div>
  );
}
