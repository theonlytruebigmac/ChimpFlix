import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminPrivacyClient } from "@/components/admin/AdminPrivacyClient";

export default async function AdminPrivacyPage() {
  const initial = await adminApi.privacy.get();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Privacy"
        description="Telemetry preferences. ChimpFlix collects nothing today; this toggle records the choice so future builds can honor it."
      />
      <AdminPrivacyClient initial={initial.telemetry_opt_in} />
    </div>
  );
}
