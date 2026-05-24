import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminLibrarySettingsClient } from "@/components/admin/AdminLibrarySettingsClient";

export default async function AdminLibrarySettingsPage() {
  const settingsRes = await adminApi.settings.get();
  return <AdminLibrarySettingsClient settings={settingsRes.settings} />;
}
