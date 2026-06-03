import { requireUser } from "@/lib/chimpflix-server";
import { SettingsAccountClient } from "@/components/SettingsAccountClient";
import { formatDate, formatDateTime } from "@/lib/format";

/// Account tab — your profile, sign-in security, sessions, and connected
/// identity providers. Rendered in the console design language to match
/// the redesign mockup (docs/redesign/account.html): in-page tabs over
/// cards + setting rows. All working logic lives in the per-feature
/// Settings*Client components; this server component only fetches the
/// user and the read-only membership facts.
export default async function AccountSettingsPage() {
  const user = await requireUser("/settings/account");

  const previousSignIn =
    user.previous_login_at != null
      ? `${formatDateTime(user.previous_login_at)}${
          user.previous_login_ip ? ` · ${user.previous_login_ip}` : ""
        }`
      : null;

  return (
    <SettingsAccountClient
      user={user}
      membership={{
        username: user.username,
        roleLabel: user.role === "owner" ? "Owner" : user.role === "admin" ? "Admin" : "User",
        isOwner: user.role === "owner",
        joined: formatDate(user.created_at),
        previousSignIn,
      }}
    />
  );
}
