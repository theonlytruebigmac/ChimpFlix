import { AdminAccessGroupsClient } from "@/components/admin/AdminAccessGroupsClient";
import {
  admin as adminApi,
  auth as authApi,
  libraries as librariesApi,
} from "@/lib/chimpflix-api";
import { requireOwner } from "@/lib/chimpflix-server";

export default async function AdminGroupsPage() {
  await requireOwner("/settings/admin/users/access-groups");
  const [{ groups }, { users }, { libraries }] = await Promise.all([
    adminApi.accessGroups.list(),
    authApi.listUsers(),
    librariesApi.list(),
  ]);
  // Filter out the placeholder _default user so admins don't see it
  // as a membership target.
  const realUsers = users.filter((u) => u.username !== "_default");

  return (
    <AdminAccessGroupsClient
      initialGroups={groups}
      users={realUsers}
      libraries={libraries}
    />
  );
}
