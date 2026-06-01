import { notFound } from "next/navigation";

import {
  admin as adminApi,
  ChimpFlixApiError,
  type KindDetailResponse,
} from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTaskDetailClient } from "@/components/admin/AdminTaskDetailClient";

/// Per-kind drill-in. Renders the schedule + gate config, live counters, and
/// a 30-day history chart for one task kind. A 404 from the backend (unknown
/// kind name) cascades into Next's notFound() so the URL is honest; any other
/// failure propagates as a real error. (Was under /scheduled-tasks/kind.)
export default async function AdminTaskDetailPage({
  params,
}: {
  params: Promise<{ kind: string }>;
}) {
  const { kind } = await params;
  let initial: KindDetailResponse;
  try {
    initial = await adminApi.tasks.detail(kind);
  } catch (e) {
    if (e instanceof ChimpFlixApiError && e.status === 404) {
      notFound();
    }
    throw e;
  }
  // Server component renders once per request.
  // eslint-disable-next-line react-hooks/purity
  const initialNowMs = Date.now();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Tasks · Detail"
        title={initial.display_name}
        description="Schedule, gate, live counters, and 30-day history for one task kind."
      />
      <AdminTaskDetailClient initial={initial} initialNowMs={initialNowMs} />
    </div>
  );
}
