import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminAccessClient } from "@/components/admin/AdminAccessClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminAccessPage() {
  const initial = await adminApi.access.get();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Access"
        description="Library × user access matrix. Toggle which non-owner users can browse each library; owners always see everything."
      />
      <AdminAccessClient initial={initial.entries} />
    </div>
  );
}
