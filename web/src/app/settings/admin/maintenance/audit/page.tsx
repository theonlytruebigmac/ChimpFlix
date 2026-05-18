import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminAuditClient } from "@/components/admin/AdminAuditClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminAuditPage() {
  const initial = await adminApi.audit.list({ limit: 50 });
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Audit Log"
        description="Administrative actions recorded with actor, target, and payload diff. Play/scrobble events are tracked separately and are not included here."
      />
      <AdminAuditClient initial={initial} />
    </div>
  );
}
