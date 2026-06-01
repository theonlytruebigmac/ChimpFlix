import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminNotificationsTabs } from "@/components/admin/AdminNotificationsTabs";

/// Notifications — SMTP email config + webhook endpoints, as tabs. Folds the
/// old server/notifications + notifications/webhooks pages.
export default async function AdminNotificationsPage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const sp = await searchParams;
  const initialTab = sp?.tab === "webhooks" ? "webhooks" : "email";
  const [email, webhooks] = await Promise.all([
    adminApi.email.status(),
    adminApi.webhooks.list(),
  ]);
  return (
    <AdminNotificationsTabs
      initialTab={initialTab}
      email={email}
      webhooks={webhooks}
    />
  );
}
