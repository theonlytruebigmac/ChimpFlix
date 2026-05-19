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
        description="Library × user access matrix. Toggle the checkboxes to grant or revoke direct access. Access granted via a group (e.g. Friends) shows beneath the cell as “via …” and is managed under Settings → Users → Groups. Owners always see everything."
      />
      <AdminAccessClient initial={initial.entries} />
    </div>
  );
}
