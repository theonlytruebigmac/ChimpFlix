import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLogsClient } from "@/components/admin/AdminLogsClient";

export default async function AdminLogsPage() {
  const initial = await adminApi.logs({ limit: 200 });
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Logs</h1>
        <p className="mt-1 text-sm text-white/60">
          Live tail from the server&apos;s tracing buffer (last ~5,000 lines).
          Filter by minimum severity; refreshes every 3 seconds.
        </p>
      </header>
      <AdminLogsClient initial={initial.lines} />
    </div>
  );
}
