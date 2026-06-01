import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminTasksTabs } from "@/components/admin/AdminTasksTabs";
import { AdminTasksOverviewClient } from "@/components/admin/AdminTasksOverviewClient";
import { AdminJobsClient } from "@/components/admin/AdminJobsClient";
import { AdminTasksActivityClient } from "@/components/admin/AdminTasksActivityClient";
import { AdminTaskFlowClient } from "@/components/admin/AdminTaskFlowClient";

export const dynamic = "force-dynamic";

const TAB_IDS = ["overview", "queue", "activity", "flow"];

/// Tasks & jobs — the background pipeline. Overview, durable job Queue, live
/// Activity feed, and the ingest Flow diagram, consolidated into one tabbed
/// page. (Folds the old /settings/admin/library/scheduled-tasks subtree.)
export default async function AdminTasksPage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const sp = await searchParams;
  const initialTab = TAB_IDS.includes(sp?.tab ?? "")
    ? (sp.tab as string)
    : "overview";

  const [overview, summary, settingsRes, activity, jobsSummary, jobsList] =
    await Promise.all([
      adminApi.tasks.overview(),
      adminApi.tasks.summary(),
      adminApi.settings.get(),
      adminApi.tasks.activity(),
      adminApi.jobs
        .summary()
        .catch(() => ({ queued: 0, running: 0, succeeded: 0, failed: 0, dead: 0 })),
      adminApi.jobs.list({ limit: 100 }).catch(() => ({ jobs: [] })),
    ]);
  // Server component renders once per request — see the impurity-rule
  // rationale carried over from the old scheduled-tasks page.
  // eslint-disable-next-line react-hooks/purity
  const initialNowMs = Date.now();

  return (
    <AdminTasksTabs
      initialTab={initialTab}
      overview={
        <AdminTasksOverviewClient
          initialOverview={overview}
          initialSummary={summary}
          initialSettings={settingsRes.settings}
          initialNowMs={initialNowMs}
        />
      }
      queue={
        <AdminJobsClient
          initialSummary={jobsSummary}
          initialJobs={jobsList.jobs}
        />
      }
      activity={
        <AdminTasksActivityClient
          initialActivity={activity}
          initialSummary={summary}
          initialNowMs={initialNowMs}
          initialKindConcurrency={settingsRes.settings.job_kind_concurrency}
        />
      }
      flow={<AdminTaskFlowClient initial={overview} />}
    />
  );
}
