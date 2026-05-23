/// Barrel for the admin design-system primitives. Every primitive is
/// re-exported here so admin pages can `import { Pill, SettingsCard,
/// SaveBar } from "@/components/admin/ui"` instead of cherry-picking
/// files. Add a new export here when you build a new primitive.

export { Pill, StatusDot, type PillTone } from "./Pill";
export { SettingsCard, SettingsRow } from "./SettingsCard";
export { SaveBar } from "./SaveBar";
export {
  Drawer,
  DrawerHeader,
  DrawerTabs,
  DrawerBody,
  DrawerKV,
  DrawerSection,
  type DrawerTab,
} from "./Drawer";
export { HeroCard } from "./HeroCard";
export { FilterChip } from "./FilterChip";
export { AdminTabBar, type AdminTab } from "./TabBar";
// Pagination lives in the shared ui/ folder so user-facing grids
// (/library/[id]/browse, /genre, /collection, /history) can reuse it
// without dipping into the admin namespace. Re-exported here so
// existing `from "./ui"` admin imports keep working.
export { Pagination, DEFAULT_PAGE_SIZE } from "@/components/ui/Pagination";
