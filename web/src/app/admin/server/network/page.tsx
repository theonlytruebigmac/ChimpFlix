import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminNetworkClient } from "@/components/admin/AdminNetworkClient";

export default async function AdminNetworkPage() {
  const initial = await adminApi.network.get();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Network</h1>
        <p className="mt-1 text-sm text-white/60">
          Public URL, CORS, and secure-connection policy. Settings apply
          without restart via the in-memory settings cache.
        </p>
      </header>
      <AdminNetworkClient initial={initial} />
    </div>
  );
}
