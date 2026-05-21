import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTasksActivityClient } from "@/components/admin/AdminTasksActivityClient";

/// Activity screen — live per-kind health + recent-runs feed +
/// dead-letter failure log. Powered by `/admin/tasks/activity` with
/// 5-second polling.
export default async function AdminTasksActivityPage() {
  const [activity, summary, settingsRes] = await Promise.all([
    adminApi.tasks.activity(),
    adminApi.tasks.summary(),
    adminApi.settings.get(),
  ]);
  // Server component renders once per request — see scheduled-
  // tasks/page.tsx for the impurity-rule rationale.
  // eslint-disable-next-line react-hooks/purity
  const initialNowMs = Date.now();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library · Activity"
        title="Task activity"
        description="Live queue depth, recent runs, and the failure log. Auto-refreshes every 5 seconds."
      />
      <AdminTasksActivityClient
        initialActivity={activity}
        initialSummary={summary}
        initialNowMs={initialNowMs}
        initialKindConcurrency={settingsRes.settings.job_kind_concurrency}
      />
    </div>
  );
}
