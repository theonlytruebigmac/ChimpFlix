import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminCredentialsClient } from "@/components/admin/AdminCredentialsClient";

/// API credentials for metadata agents & integrations. (Flattened up from
/// /settings/admin/server/credentials.)
export default async function AdminCredentialsPage() {
  const initial = await adminApi.secrets.list();
  return (
    <div>
      <p className="mb-5 max-w-2xl text-sm text-white/55">
        API keys for metadata agents and other integrations. Values are
        encrypted at rest when{" "}
        <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
          CHIMPFLIX_SECRET_KEY
        </code>{" "}
        is set in the server&rsquo;s environment.
      </p>
      <AdminCredentialsClient initial={initial} />
    </div>
  );
}
