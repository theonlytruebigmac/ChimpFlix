import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminAuditClient } from "@/components/admin/AdminAuditClient";

export default async function AdminAuditPage() {
  const initial = await adminApi.audit.list({ limit: 50 });
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Audit Log</h1>
        <p className="mt-1 text-sm text-white/60">
          Administrative actions recorded with actor, target, and payload
          diff. Play/scrobble events are tracked separately and are not
          included here.
        </p>
      </header>
      <AdminAuditClient initial={initial} />
    </div>
  );
}
