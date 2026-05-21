import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminAuditClient } from "@/components/admin/AdminAuditClient";

export default async function AdminAuditPage() {
  const initial = await adminApi.audit.list({ limit: 50 });
  return <AdminAuditClient initial={initial} />;
}
