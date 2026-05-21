import { requireAdmin } from "@/lib/chimpflix-server";
import { AdminUsersUnifiedClient } from "@/components/admin/AdminUsersUnifiedClient";

/// Default Users tab — the People list. Admins reach it to manage
/// users + other admins; the drawer's per-row actions gate on the
/// actor's role so an admin never sees mutate-an-owner buttons.
export default async function AdminUsersPage() {
  const actor = await requireAdmin("/settings/admin/users");
  return (
    <AdminUsersUnifiedClient
      currentUserId={actor.id}
      currentUserRole={actor.role}
    />
  );
}
