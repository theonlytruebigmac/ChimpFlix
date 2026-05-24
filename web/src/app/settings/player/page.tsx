import { SettingsPlayerClient } from "@/components/SettingsPlayerClient";
import { requireUser } from "@/lib/chimpflix-server";
import { subtitleStyleFromUser } from "@/lib/subtitle-style";

export default async function PlayerSettingsPage() {
  // Subtitle styling is server-synced per account (phase 89). Fetch the
  // user once on the server, seed the client with the canonical style;
  // the panel mutates it locally + PATCHes /auth/me. A refresh re-reads
  // from the server, so cross-device changes are picked up here too.
  const user = await requireUser("/settings/player");
  const initialSubtitleStyle = subtitleStyleFromUser(user);
  return (
    <div className="divide-y divide-white/10">
      <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
          Player
        </h2>
        <div>
          <SettingsPlayerClient initialSubtitleStyle={initialSubtitleStyle} />
        </div>
      </section>
    </div>
  );
}
