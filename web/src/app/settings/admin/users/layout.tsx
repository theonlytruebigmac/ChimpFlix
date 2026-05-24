import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTabBar } from "@/components/admin/ui";

const USERS_TABS = [
  { href: "/settings/admin/users", label: "People" },
  { href: "/settings/admin/users/invites", label: "Invites" },
  { href: "/settings/admin/users/access", label: "Access" },
  { href: "/settings/admin/users/access-groups", label: "Groups" },
  { href: "/settings/admin/users/devices", label: "Devices" },
];

/// Shared shell for everything under `/settings/admin/users`. The
/// five surfaces (people / invites / access / groups / devices)
/// used to be top-level nav rows; consolidated into a single
/// "Users" sidebar entry with tabs here so the IA reflects "one
/// concern, several views" instead of five sibling concepts.
///
/// Each sub-page is still a real Next.js route — fetches its own
/// data server-side, keeps its own auth gate — but they share this
/// header + tab bar so the visual frame stays consistent.
export default function AdminUsersLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Users"
        title="Users"
        description="People, invites, library access, groups, and active devices. Each tab focuses one part of identity & permissions."
      />
      <AdminTabBar tabs={USERS_TABS} />
      {children}
    </div>
  );
}
