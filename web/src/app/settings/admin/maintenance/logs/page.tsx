import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLogsClient } from "@/components/admin/AdminLogsClient";

export default async function AdminLogsPage() {
  // Fetch with the same `level=INFO` floor the client picks on
  // mount. Without it the server sends every level (including
  // TRACE/DEBUG), the page paints them on first frame, then the
  // client's first effect re-fetches at INFO and the unfiltered
  // noise disappears in a visible flash. Matching the default
  // here makes the flash go away.
  const initial = await adminApi.logs({ level: "INFO", limit: 200 });
  return <AdminLogsClient initial={initial.lines} />;
}
