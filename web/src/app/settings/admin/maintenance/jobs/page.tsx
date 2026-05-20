import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminJobsClient } from "@/components/admin/AdminJobsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminJobsPage() {
  // Server-render the initial summary + recent jobs so the page is
  // useful immediately on first paint. The client refreshes on a
  // short interval so the operator can leave it open as a status
  // panel during a backlog drain.
  const [summary, list] = await Promise.all([
    adminApi.jobs.summary().catch(() => ({
      queued: 0,
      running: 0,
      succeeded: 0,
      failed: 0,
      dead: 0,
    })),
    adminApi.jobs.list({ limit: 100 }).catch(() => ({ jobs: [] })),
  ]);
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Job queue"
        description="Durable background pipeline — marker detection, preview sprites, loudness, chapter thumbs. Discovery-triggered: every new media file enqueues the full chain. Scheduled tasks now act as safety nets only."
      />
      <AdminJobsClient initialSummary={summary} initialJobs={list.jobs} />
    </div>
  );
}
