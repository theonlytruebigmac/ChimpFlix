import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminAccessClient } from "@/components/admin/AdminAccessClient";

export default async function AdminAccessPage() {
  const initial = await adminApi.access.get();
  return <AdminAccessClient initial={initial.entries} />;
}
