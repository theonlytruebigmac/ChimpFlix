import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminDashboardClient } from "@/components/admin/AdminDashboardClient";

/// Server overview — system health, live sessions, recent activity, and
/// alerts at a glance, with quick actions. This is the admin landing page
/// (was /settings/admin). No page-title header: the sidebar's active item
/// and the breadcrumb identify the page.
export default async function AdminOverviewPage() {
  const initial = await adminApi.dashboard();
  return <AdminDashboardClient initial={initial} />;
}
