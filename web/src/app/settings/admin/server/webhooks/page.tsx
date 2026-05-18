import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminWebhooksClient } from "@/components/admin/AdminWebhooksClient";

export default async function AdminWebhooksPage() {
  const initial = await adminApi.webhooks.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Webhooks"
        description="Outbound HTTP notifications. Payloads are signed with HMAC-SHA256 when a secret is set; failures retry up to 3 times."
      />
      <AdminWebhooksClient initial={initial} />
    </div>
  );
}
