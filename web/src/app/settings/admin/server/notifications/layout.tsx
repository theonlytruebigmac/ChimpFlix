import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTabBar } from "@/components/admin/ui";

const NOTIFICATIONS_TABS = [
  { href: "/settings/admin/server/notifications", label: "Email (SMTP)" },
  {
    href: "/settings/admin/server/notifications/webhooks",
    label: "Webhooks",
  },
];

/// Outbound-notification configuration: email relay + webhooks.
/// Used to be two separate top-level Server entries; consolidated
/// here because they answer the same operator question ("how does
/// ChimpFlix reach the outside world?") and share the same
/// credential-vault + retry semantics on the backend.
export default function AdminNotificationsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Notifications"
        description="Outbound delivery — SMTP for transactional email, webhooks for fan-out to external systems. Both share the encrypted credential vault."
      />
      <AdminTabBar tabs={NOTIFICATIONS_TABS} />
      {children}
    </div>
  );
}
