import { SettingsPlayerClient } from "@/components/SettingsPlayerClient";
import { requireUser } from "@/lib/chimpflix-server";
import { subtitleStyleFromUser } from "@/lib/subtitle-style";

export default async function PlaybackSettingsPage() {
  // Subtitle styling is server-synced per account (phase 89). Seed the
  // client with the canonical style; the panel mutates it locally +
  // PATCHes /auth/me, so a refresh re-reads the server value and
  // cross-device changes show up here too. Device-local playback
  // toggles live in localStorage via usePrefs inside the client.
  const user = await requireUser("/settings/playback");
  return (
    <SettingsPlayerClient initialSubtitleStyle={subtitleStyleFromUser(user)} />
  );
}
