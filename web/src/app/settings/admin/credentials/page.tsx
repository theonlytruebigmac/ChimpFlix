import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminCredentialsClient } from "@/components/admin/AdminCredentialsClient";

/// API credentials for metadata agents & integrations. (Flattened up from
/// /settings/admin/server/credentials.) Rendered in the console design
/// language — page head, encrypted-at-rest banner, and per-provider cards
/// all live in the client component.
export default async function AdminCredentialsPage() {
  const initial = await adminApi.secrets.list();
  return <AdminCredentialsClient initial={initial} />;
}
