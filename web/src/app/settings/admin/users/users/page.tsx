import { requireOwner } from "@/lib/chimpflix-server";
import { SettingsUsersClient } from "@/components/SettingsUsersClient";

export default async function AdminUsersPage() {
  const owner = await requireOwner("/admin/users/users");
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Users</h1>
        <p className="mt-1 text-sm text-white/60">
          Owner and viewer accounts. Promote/demote and delete; you cannot
          modify your own role here.
        </p>
      </header>
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsUsersClient currentUserId={owner.id} />
      </section>
    </div>
  );
}
