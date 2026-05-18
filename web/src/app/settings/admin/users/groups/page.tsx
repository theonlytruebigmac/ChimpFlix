import { AdminAccessGroupsClient } from "@/components/admin/AdminAccessGroupsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import {
  admin as adminApi,
  auth as authApi,
  libraries as librariesApi,
} from "@/lib/chimpflix-api";
import { requireOwner } from "@/lib/chimpflix-server";

export default async function AdminGroupsPage() {
  await requireOwner("/settings/admin/users/groups");
  const [{ groups }, { users }, { libraries }] = await Promise.all([
    adminApi.accessGroups.list(),
    authApi.listUsers(),
    librariesApi.list(),
  ]);
  // Filter out the placeholder _default user so admins don't see it as
  // a membership target.
  const realUsers = users.filter((u) => u.username !== "_default");

  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Access groups"
        description="Bundle library permissions and assign them to many users at once. A user's effective access is the union of direct grants and every group they belong to."
      />
      <AdminAccessGroupsClient
        initialGroups={groups}
        users={realUsers}
        libraries={libraries}
      />
    </div>
  );
}
