import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminNetworkClient } from "@/components/admin/AdminNetworkClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminNetworkPage() {
  const initial = await adminApi.network.get();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Network"
        description="Public URL, CORS, and secure-connection policy. Settings apply without restart via the in-memory settings cache."
      />
      <AdminNetworkClient initial={initial} />
    </div>
  );
}
