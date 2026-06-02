import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminGeneralForm } from "@/components/admin/AdminGeneralForm";

/// Server identity & privacy. (Flattened up from /settings/admin/server/general.)
export default async function AdminGeneralPage() {
  const { settings, version, data_dir } = await adminApi.settings.get();
  return (
    <AdminGeneralForm initial={settings} version={version} dataDir={data_dir} />
  );
}
