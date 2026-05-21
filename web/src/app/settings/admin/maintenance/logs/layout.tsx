import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTabBar } from "@/components/admin/ui";

const LOG_TABS = [
  { href: "/settings/admin/maintenance/logs", label: "Application" },
  { href: "/settings/admin/maintenance/logs/audit", label: "Audit trail" },
];

/// Shared shell for everything log-shaped: live application tail
/// (tracing buffer) and the admin audit trail. Both used to live as
/// separate nav rows under Maintenance — collapsed here since they
/// answer the same operator question ("what happened?") and share
/// the same filter-by-time + filter-by-level mental model.
export default function AdminLogsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Logs"
        description="Live tail from the server's tracing buffer plus the audit trail of administrative actions. Switch tabs to flip between technical and policy-level events."
      />
      <AdminTabBar tabs={LOG_TABS} />
      {children}
    </div>
  );
}
