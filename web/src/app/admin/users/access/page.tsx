import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminAccessClient } from "@/components/admin/AdminAccessClient";

export default async function AdminAccessPage() {
  const initial = await adminApi.access.get();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Access</h1>
        <p className="mt-1 text-sm text-white/60">
          Library × user access matrix. Toggle which non-owner users can
          browse each library; owners always see everything.
        </p>
      </header>
      <AdminAccessClient initial={initial.entries} />
    </div>
  );
}
