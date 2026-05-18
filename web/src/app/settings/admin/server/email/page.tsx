import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminEmailClient } from "@/components/admin/AdminEmailClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminEmailPage() {
  const status = await adminApi.email.status();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Email"
        description="SMTP relay used for invites, password resets, and admin notifications. The password is held in the encrypted credential vault and never returned by the API."
      />
      <AdminEmailClient initial={status} />
    </div>
  );
}
