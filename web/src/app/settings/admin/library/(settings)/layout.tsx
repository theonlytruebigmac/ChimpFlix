import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { AdminTabBar } from "@/components/admin/ui";

const LIBRARY_SETTINGS_TABS = [
  { href: "/settings/admin/library", label: "Settings" },
  { href: "/settings/admin/library/preroll", label: "Pre-roll" },
  { href: "/settings/admin/library/bulk", label: "Bulk operations" },
];

/// Layout for the Library Settings tabbed surface — only wraps the
/// route-group siblings (settings index, pre-roll, bulk operations).
/// Sibling routes like `library/libraries`, `library/collections`,
/// `library/scheduled-tasks` aren't inside this `(settings)` group
/// and stay on their own — they're distinct resources, not "library
/// settings" in the configuration sense.
export default function AdminLibrarySettingsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Library Settings"
        description="Server-wide library tuning, the pre-roll trailer, and the multi-select bulk-operations tool. Per-library knobs (paths, visibility, deletions) live under Library → Libraries."
      />
      <AdminTabBar tabs={LIBRARY_SETTINGS_TABS} />
      {children}
    </div>
  );
}
