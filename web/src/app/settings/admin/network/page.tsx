import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminNetworkClient } from "@/components/admin/AdminNetworkClient";

/// Network & access policy. (Flattened up from /settings/admin/server/network.)
export default async function AdminNetworkPage() {
  const initial = await adminApi.network.get();
  return <AdminNetworkClient initial={initial} />;
}
