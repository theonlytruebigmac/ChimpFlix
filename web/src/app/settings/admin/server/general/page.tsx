import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminGeneralForm } from "@/components/admin/AdminGeneralForm";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminGeneralPage() {
  const { settings } = await adminApi.settings.get();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="General"
        description="Server-wide identity and privacy controls. Detailed transcoder, network, and webhook settings live on their own pages."
      />
      <AdminGeneralForm initial={settings} />
    </div>
  );
}
