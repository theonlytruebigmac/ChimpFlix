import { redirect } from "next/navigation";

export default function SettingsIndexPage() {
  // The unified shell defaults to the Account tab. Sub-pages under
  // /settings/* render via the shared layout.
  redirect("/settings/account");
}
