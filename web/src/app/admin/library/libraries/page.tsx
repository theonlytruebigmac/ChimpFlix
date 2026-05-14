import { libraries as librariesApi, admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLibrariesClient } from "@/components/admin/AdminLibrariesClient";

export default async function AdminLibrariesPage() {
  const [libs, agents] = await Promise.all([
    librariesApi.list(),
    adminApi.agents.list(),
  ]);
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Libraries</h1>
        <p className="mt-1 text-sm text-white/60">
          Paths, scan schedule, episode ordering, certification country, and
          metadata agent priority per library.
        </p>
      </header>
      <AdminLibrariesClient
        initialLibraries={libs.libraries}
        availableAgents={agents.agents}
      />
    </div>
  );
}
