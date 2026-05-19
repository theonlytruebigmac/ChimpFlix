import { AdminBackupRestoreClient } from "@/components/admin/AdminBackupRestoreClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { SettingsBackupClient } from "@/components/SettingsBackupClient";

export default async function AdminBackupPage() {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Backup & Restore"
        description={
          <>
            On-demand SQLite snapshot via{" "}
            <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
              VACUUM INTO
            </code>
            . Auto-backups (written by the{" "}
            <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
              backup_db
            </code>{" "}
            scheduled task) persist under{" "}
            <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
              &lt;data_dir&gt;/backups/auto/
            </code>{" "}
            and can be restored from the list below.
          </>
        }
      />
      <div className="space-y-6">
        <section className="rounded-lg border border-white/10 bg-white/2 p-6">
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-white/40">
            One-shot download
          </h2>
          <SettingsBackupClient />
        </section>
        <AdminBackupRestoreClient />
      </div>
    </div>
  );
}
