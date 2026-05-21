import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTasksOverviewClient } from "@/components/admin/AdminTasksOverviewClient";

/// Scheduled-tasks admin page. Registry-driven overview is the
/// only view — the legacy `?advanced=1` per-row CRUD editor was
/// folded into the new detail page (Schedule card now edits
/// frequency/enabled/window) and the maintenance-window editor
/// moved up here, so there's nothing left to hide behind an
/// "Advanced" toggle.
export default async function AdminTasksPage() {
  const [overview, summary, settings] = await Promise.all([
    adminApi.tasks.overview(),
    adminApi.tasks.summary(),
    adminApi.settings.get(),
  ]);
  // Snapshot `now` after the fetches so the relative-time
  // formatters in the client get a consistent reference instant
  // for the initial server render. The client re-snapshots on
  // every refresh — this just keeps SSR HTML and the first
  // client paint in agreement so React doesn't have to rewrite
  // the DOM on hydration. Server components render once at
  // request time, not on every state change, so the impurity
  // rule doesn't apply the way it does to client renders.
  // eslint-disable-next-line react-hooks/purity
  const initialNowMs = Date.now();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Scheduled Tasks"
        description="Background pipelines that keep libraries fresh, detect intros &amp; credits, and maintain system health. Gated tasks are off by default — flip them on per feature."
      />
      <AdminTasksOverviewClient
        initialOverview={overview}
        initialSummary={summary}
        initialSettings={settings.settings}
        initialNowMs={initialNowMs}
      />
    </div>
  );
}
