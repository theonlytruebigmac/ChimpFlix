import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminDevicesClient } from "@/components/admin/AdminDevicesClient";

export default async function AdminDevicesPage() {
  const initial = await adminApi.sessions.list();
  return <AdminDevicesClient initial={initial.sessions} />;
}
