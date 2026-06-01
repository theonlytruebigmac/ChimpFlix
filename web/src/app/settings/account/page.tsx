import { server as serverApi } from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";
import { SettingsEmailChangeClient } from "@/components/SettingsEmailChangeClient";
import { SettingsLinkedAccountsClient } from "@/components/SettingsLinkedAccountsClient";
import { SettingsPasswordClient } from "@/components/SettingsPasswordClient";
import { SettingsProfileClient } from "@/components/SettingsProfileClient";
import { SettingsTwoFactorClient } from "@/components/SettingsTwoFactorClient";
import { formatDate, formatDateTime } from "@/lib/format";

/// Account tab. Holds the per-user identity surface plus the
/// server-info summary that used to sit at the top of the old settings
/// page. Account-style layout: flat sections separated by hairlines,
/// label-value rows right-aligned. No surface card chrome.
export default async function AccountSettingsPage() {
  const user = await requireUser("/settings/account");
  const info = await serverApi.info();

  return (
    <div className="divide-y divide-white/10">
      <Section title="Profile">
        <SettingsProfileClient initial={user} />
      </Section>

      <Section title="Membership">
        <Row label="Username" value={`@${user.username}`} />
        <Row label="Role" value={user.role === "owner" ? "Owner" : "User"} />
        <Row label="Joined" value={formatDate(user.created_at)} />
        {user.previous_login_at != null && (
          <Row
            label="Previous sign-in"
            value={`${formatDateTime(user.previous_login_at)}${
              user.previous_login_ip ? ` from ${user.previous_login_ip}` : ""
            }`}
          />
        )}
      </Section>

      <Section title="Email">
        <SettingsEmailChangeClient initial={user} />
      </Section>

      <Section title="Password">
        <SettingsPasswordClient />
      </Section>

      <Section title="Two-factor">
        <SettingsTwoFactorClient />
      </Section>

      <Section title="Linked accounts">
        <SettingsLinkedAccountsClient />
      </Section>

      <Section title="Server">
        <Row label="Version" value={`v${info.version}`} />
        <Row label="Libraries" value={`${info.library_counts.libraries}`} />
        <Row label="Movies" value={`${info.library_counts.movies}`} />
        <Row label="Shows" value={`${info.library_counts.shows}`} />
        <Row label="Episodes" value={`${info.library_counts.episodes}`} />
        <Row
          label="TMDB enrichment"
          value={info.tmdb_enabled ? "Enabled" : "Disabled"}
        />
      </Section>
    </div>
  );
}

/// Account-style section: no border/background, generous vertical
/// padding, a small uppercase heading hugging the left edge. Looks
/// like netflix.com/account rather than a settings card.
function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
      <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
        {title}
      </h2>
      <div className="space-y-2">{children}</div>
    </section>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-4 text-sm">
      <span className="text-white/55">{label}</span>
      <span className="text-white">{value}</span>
    </div>
  );
}
