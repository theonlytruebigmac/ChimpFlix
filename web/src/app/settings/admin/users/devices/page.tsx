import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminDevicesClient } from "@/components/admin/AdminDevicesClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminDevicesPage() {
  const initial = await adminApi.sessions.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Devices"
        description="Active browser sessions across all users. Revoking a session logs that browser out on its next request."
      />
      <AdminDevicesClient initial={initial.sessions} />
    </div>
  );
}
