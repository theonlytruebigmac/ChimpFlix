import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminPrivacyClient } from "@/components/admin/AdminPrivacyClient";

export default async function AdminPrivacyPage() {
  const initial = await adminApi.privacy.get();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Privacy</h1>
        <p className="mt-1 text-sm text-white/60">
          Telemetry preferences. ChimpFlix collects nothing today; this
          toggle records the choice so future builds can honor it.
        </p>
      </header>
      <AdminPrivacyClient initial={initial.telemetry_opt_in} />
    </div>
  );
}
