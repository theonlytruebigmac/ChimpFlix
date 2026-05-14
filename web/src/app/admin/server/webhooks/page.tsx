import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminWebhooksClient } from "@/components/admin/AdminWebhooksClient";

export default async function AdminWebhooksPage() {
  const initial = await adminApi.webhooks.list();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Webhooks</h1>
        <p className="mt-1 text-sm text-white/60">
          Outbound HTTP notifications. Payloads are signed with HMAC-SHA256
          when a secret is set; failures retry up to 3 times.
        </p>
      </header>
      <AdminWebhooksClient initial={initial} />
    </div>
  );
}
