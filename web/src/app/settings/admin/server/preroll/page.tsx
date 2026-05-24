import {
  admin as adminApi,
  preroll as prerollApi,
} from "@/lib/chimpflix-api";
import { AdminPrerollClient } from "@/components/admin/AdminPrerollClient";

export default async function AdminPrerollPage() {
  const [status, { settings }] = await Promise.all([
    prerollApi.status(),
    adminApi.settings.get(),
  ]);
  return (
    <AdminPrerollClient
      initialStatus={status}
      initialEnabled={settings.preroll_enabled}
      initialVolume={settings.preroll_volume ?? 100}
    />
  );
}
