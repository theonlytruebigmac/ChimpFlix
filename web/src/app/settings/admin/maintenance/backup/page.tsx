import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { SettingsBackupClient } from "@/components/SettingsBackupClient";

export default async function AdminBackupPage() {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Backup"
        description={
          <>
            On-demand SQLite snapshot via{" "}
            <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
              VACUUM INTO
            </code>
            . The server writes a self-contained .db file and streams it
            back as a download.
          </>
        }
      />
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsBackupClient />
      </section>
    </div>
  );
}
