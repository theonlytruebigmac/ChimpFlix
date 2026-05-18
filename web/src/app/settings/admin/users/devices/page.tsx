import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminDevicesClient } from "@/components/admin/AdminDevicesClient";

export default async function AdminDevicesPage() {
  const initial = await adminApi.sessions.list();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Devices</h1>
        <p className="mt-1 text-sm text-white/60">
          Active browser sessions across all users. Revoking a session
          logs that browser out on its next request.
        </p>
      </header>
      <AdminDevicesClient initial={initial.sessions} />
    </div>
  );
}
