import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminEmailClient } from "@/components/admin/AdminEmailClient";

export default async function AdminNotificationsEmailPage() {
  const status = await adminApi.email.status();
  return <AdminEmailClient initial={status} />;
}
