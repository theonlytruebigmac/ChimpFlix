import { libraries as librariesApi } from "@/lib/chimpflix-api";
import { AdminBulkItemsClient } from "@/components/admin/AdminBulkItemsClient";

export default async function AdminBulkItemsPage() {
  const { libraries } = await librariesApi.list();
  return <AdminBulkItemsClient libraries={libraries} />;
}
