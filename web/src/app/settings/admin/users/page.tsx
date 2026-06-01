import {
  admin as adminApi,
  auth as authApi,
  libraries as librariesApi,
} from "@/lib/chimpflix-api";
import { requireAdmin } from "@/lib/chimpflix-server";
import { AdminUsersTabs } from "@/components/admin/AdminUsersTabs";
import { AdminUsersUnifiedClient } from "@/components/admin/AdminUsersUnifiedClient";
import { AdminAccessClient } from "@/components/admin/AdminAccessClient";
import { AdminAccessGroupsClient } from "@/components/admin/AdminAccessGroupsClient";
import { AdminDevicesClient } from "@/components/admin/AdminDevicesClient";
import { SettingsInvitesClient } from "@/components/SettingsInvitesClient";

const TAB_IDS = ["people", "invites", "access", "groups", "devices"];

/// Users & access — people, invites, the per-library access matrix, access
/// groups, and active devices, consolidated into one tabbed page (was the
/// /settings/admin/users subtree). The People drawer gates per-row actions on
/// the actor's role, same as before.
export default async function AdminUsersPage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const actor = await requireAdmin("/settings/admin/users");
  const sp = await searchParams;
  const initialTab = TAB_IDS.includes(sp?.tab ?? "")
    ? (sp.tab as string)
    : "people";

  const [accessRes, groupsRes, usersRes, libsRes, sessionsRes] =
    await Promise.all([
      adminApi.access.get(),
      adminApi.accessGroups.list(),
      authApi.listUsers(),
      librariesApi.list(),
      adminApi.sessions.list(),
    ]);
  // Drop the placeholder _default user so it isn't a membership target.
  const realUsers = usersRes.users.filter((u) => u.username !== "_default");

  return (
    <AdminUsersTabs
      initialTab={initialTab}
      people={
        <AdminUsersUnifiedClient
          currentUserId={actor.id}
          currentUserRole={actor.role}
        />
      }
      invites={
        <section className="rounded-lg border border-white/10 bg-white/2 p-6">
          <SettingsInvitesClient />
        </section>
      }
      access={<AdminAccessClient initial={accessRes.entries} />}
      groups={
        <AdminAccessGroupsClient
          initialGroups={groupsRes.groups}
          users={realUsers}
          libraries={libsRes.libraries}
        />
      }
      devices={<AdminDevicesClient initial={sessionsRes.sessions} />}
    />
  );
}
