import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { SettingsInvitesClient } from "@/components/SettingsInvitesClient";

export default async function AdminInvitesPage() {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Invites"
        description="Create signup codes for new viewers. Revoke from this page to take an unused invite out of circulation."
      />
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsInvitesClient />
      </section>
    </div>
  );
}
