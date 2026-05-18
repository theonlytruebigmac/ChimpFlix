import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminGeneralForm } from "@/components/admin/AdminGeneralForm";

export default async function AdminGeneralPage() {
  const { settings } = await adminApi.settings.get();
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">General</h1>
        <p className="mt-1 text-sm text-white/60">
          Server-wide identity and privacy controls. Detailed transcoder,
          network, and webhook settings live on their own pages.
        </p>
      </header>
      <AdminGeneralForm initial={settings} />
    </div>
  );
}
