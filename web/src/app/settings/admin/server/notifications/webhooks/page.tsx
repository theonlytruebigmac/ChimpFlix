import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminWebhooksClient } from "@/components/admin/AdminWebhooksClient";

export default async function AdminNotificationsWebhooksPage() {
  const initial = await adminApi.webhooks.list();
  return <AdminWebhooksClient initial={initial} />;
}
