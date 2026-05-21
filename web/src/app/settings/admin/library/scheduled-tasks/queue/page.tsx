import Link from "next/link";
import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminJobsClient } from "@/components/admin/AdminJobsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

/// Background job queue. Previously lived at
/// `/settings/admin/maintenance/jobs`; folded into the
/// scheduled-tasks UI in the 2026-05-20 consolidation so there's a
/// single surface for everything queue/cron related. The legacy
/// route now redirects here.
export default async function AdminTasksQueuePage() {
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
        eyebrow="Library · Scheduled tasks"
        title="Job queue"
        description="Durable background pipeline — marker detection, preview sprites, loudness, chapter thumbs. Discovery-triggered: every new media file enqueues the full chain. Scheduled tasks act as safety nets only."
      />
      <div className="mb-4 flex items-center justify-end gap-2 text-xs text-white/60">
        <Link
          href="/settings/admin/library/scheduled-tasks"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          ← Tasks
        </Link>
        <Link
          href="/settings/admin/library/scheduled-tasks/activity"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          Activity
        </Link>
      </div>
      <AdminJobsClient initialSummary={summary} initialJobs={list.jobs} />
    </div>
  );
}
