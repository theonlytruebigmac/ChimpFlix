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
        description="Public URL, CORS, LAN policy, and session cleanup. Most settings apply without restart; the reaper threshold is read once at spawn time and flags itself when changed."
      />
      <AdminNetworkClient initial={initial} />
    </div>
  );
}
