import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminGeneralForm } from "@/components/admin/AdminGeneralForm";

/// Server identity & privacy. (Flattened up from /settings/admin/server/general.)
export default async function AdminGeneralPage() {
  const { settings } = await adminApi.settings.get();
  return <AdminGeneralForm initial={settings} />;
}
