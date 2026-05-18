import { libraries as librariesApi, admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLibrariesClient } from "@/components/admin/AdminLibrariesClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminLibrariesPage() {
  const [libs, agents] = await Promise.all([
    librariesApi.list(),
    adminApi.agents.list(),
  ]);
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Libraries"
        description="Paths, scan schedule, episode ordering, certification country, and metadata agent priority per library."
      />
      <AdminLibrariesClient
        initialLibraries={libs.libraries}
        availableAgents={agents.agents}
      />
    </div>
  );
}
