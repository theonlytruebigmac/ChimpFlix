import { requireAdmin } from "@/lib/chimpflix-server";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminUsersUnifiedClient } from "@/components/admin/AdminUsersUnifiedClient";

export default async function AdminUsersPage() {
  // Admins can reach this page to manage users + other admins. The
  // drawer's quick-action buttons gate by the actor's role so an
  // admin never sees a way to mutate an owner. Owner-only mutations
  // (promote to owner, manage owners) still happen here — gated by
  // the same backend hierarchy.
  const actor = await requireAdmin("/settings/admin/users/users");
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Users"
        description="Owner, admin, and viewer accounts. Click any user to open the drawer with their profile, access, devices, and audit history."
      />
      <AdminUsersUnifiedClient
        currentUserId={actor.id}
        currentUserRole={actor.role}
      />
    </div>
  );
}
