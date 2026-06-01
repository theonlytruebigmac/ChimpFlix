import { libraries as librariesApi } from "@/lib/chimpflix-api";
import { SettingsHiddenLibrariesClient } from "@/components/SettingsHiddenLibrariesClient";

/// Home & visibility — personal control over which libraries show up on your
/// home page and browse rails. (The hidden-libraries half of the old
/// /settings/libraries page; library management itself is owner-only under
/// Server → Libraries.)
export default async function HomeVisibilitySettingsPage() {
  const libsResp = await librariesApi.list();
  return (
    <div className="divide-y divide-white/10">
      <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
          Home &amp; visibility
        </h2>
        <div className="space-y-3">
          <p className="text-sm text-white/55">
            Hidden libraries are excluded from your home page and browse rails.
          </p>
          <SettingsHiddenLibrariesClient libraries={libsResp.libraries} />
        </div>
      </section>
    </div>
  );
}
