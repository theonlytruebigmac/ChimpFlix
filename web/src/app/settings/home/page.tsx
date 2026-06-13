import { libraries as librariesApi } from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";
import { SettingsHomeVisibilityClient } from "@/components/SettingsHomeVisibilityClient";

/// Home & visibility — the personal "You" surface for controlling which
/// libraries appear on your home page + browse rails, the order + on/off
/// state of the home rails themselves, and two browse filters (hide watched
/// from Continue Watching, kids-safe). Rendered in the console design
/// language to match docs/redesign/home-visibility.html; the breadcrumb +
/// active sidebar item identify the page, so there's no page-title header.
///
/// Server component: fetches the user (for the rail-layout overlay + filter
/// prefs) and the library list (for the per-library show/hide switches). All
/// state + persistence lives in the client; hidden libraries persist via the
/// /auth/me/hidden-libraries prefs endpoint, while the rail layout + filters
/// persist via PATCH /auth/me.
export default async function HomeVisibilitySettingsPage() {
  const [user, { libraries }] = await Promise.all([
    requireUser("/settings/home"),
    librariesApi.list(),
  ]);
  // Library-management visibility ("hidden" / "search_only") is the owner's
  // server-level setting; libraries the owner fully hid from the home/search
  // surface shouldn't appear as a personal show/hide toggle. Personal hiding
  // layers on top of the home-and-search and search-only ones.
  const personalLibs = libraries.filter((l) => l.visibility !== "hidden");
  return (
    <SettingsHomeVisibilityClient user={user} libraries={personalLibs} />
  );
}
