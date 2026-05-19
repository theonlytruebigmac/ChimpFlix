import { requireAdmin } from "@/lib/chimpflix-server";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { SettingsUsersClient } from "@/components/SettingsUsersClient";

export default async function AdminUsersPage() {
  // Admins can reach this page to manage users + other admins. Per-row
  // buttons inside SettingsUsersClient gate by the actor's role so an
  // admin never sees a way to mutate an owner.
  const actor = await requireAdmin("/settings/admin/users/users");
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Users"
        description="Owner, admin, and viewer accounts. Owners can manage everyone; admins can manage users + other admins but not owners. You cannot modify your own role here."
      />
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsUsersClient currentUserId={actor.id} currentUserRole={actor.role} />
      </section>
    </div>
  );
}
