import type { ReactNode } from "react";

/// Sticky right-rail drawer used by the list-detail admin pages
/// (Libraries, Users, Bulk operations). The parent supplies a 1fr +
/// 420px grid and renders <Drawer> in the right slot when a row is
/// selected. The drawer scrolls within the viewport — its parent
/// shouldn't try to constrain height.
///
/// Note: this is an in-flow drawer, not an overlay. We chose in-flow
/// because the list view and the detail view are equally useful side
/// by side; an overlay would force a focus mode and hide the rest of
/// the table when an operator is mid-bulk-edit.
export function Drawer({ children }: { children: ReactNode }) {
  return (
    <aside className="sticky top-4 self-start overflow-hidden rounded-lg border border-white/10 bg-white/2">
      {children}
    </aside>
  );
}

/// Drawer header: title row + meta-pill row. Pass a close button via
/// `onClose` and we'll render an X in the top right; the parent is
/// responsible for actually clearing the selected-row state.
export function DrawerHeader({
  children,
  onClose,
}: {
  children: ReactNode;
  onClose?: () => void;
}) {
  return (
    <header className="border-b border-white/10 px-5 pb-4 pt-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">{children}</div>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            aria-label="Close drawer"
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-white/70 hover:bg-white/5 hover:text-white"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        )}
      </div>
    </header>
  );
}

export interface DrawerTab {
  id: string;
  label: string;
  /// Optional small count rendered after the label (e.g. session
  /// count, audit-entry count). Pass `undefined` to omit.
  count?: number | string;
}

/// Tab bar that sits between the drawer header and the body. The
/// parent owns the active-id state — we just render and notify on
/// click.
export function DrawerTabs({
  tabs,
  activeId,
  onSelect,
}: {
  tabs: DrawerTab[];
  activeId: string;
  onSelect: (id: string) => void;
}) {
  return (
    <nav className="flex border-b border-white/10 px-2" role="tablist">
      {tabs.map((t) => {
        const active = t.id === activeId;
        return (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onSelect(t.id)}
            className={`flex-1 whitespace-nowrap border-b-2 px-2 py-2.5 text-[12.5px] font-medium transition-colors ${
              active
                ? "border-accent text-white"
                : "border-transparent text-white/60 hover:text-white"
            }`}
          >
            {t.label}
            {t.count !== undefined && (
              <span className="ml-1.5 text-[10.5px] text-white/45">
                {t.count}
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
}

export function DrawerBody({ children }: { children: ReactNode }) {
  return <div className="px-5 py-4 text-[12.5px]">{children}</div>;
}

/// Two-column key/value list. Used inside DrawerBody for at-a-glance
/// summaries (path, joined, last login, etc.).
export function DrawerKV({
  rows,
}: {
  rows: Array<{ label: string; value: ReactNode }>;
}) {
  return (
    <dl className="grid grid-cols-[120px_1fr] gap-x-3 gap-y-1.5">
      {rows.map((r) => (
        <div key={r.label} className="contents">
          <dt className="text-white/50">{r.label}</dt>
          <dd className="m-0 min-w-0 text-white">{r.value}</dd>
        </div>
      ))}
    </dl>
  );
}

/// Small uppercase section title used inside drawers ("Quick actions",
/// "Library access", "Scan history"). Visually distinct from page
/// headings without competing for attention.
export function DrawerSection({
  title,
  aside,
  children,
}: {
  title: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mt-4 first:mt-0">
      <div className="mb-2 flex items-baseline justify-between gap-2">
        <h4 className="m-0 text-[11px] font-semibold uppercase tracking-[0.06em] text-white/50">
          {title}
        </h4>
        {aside}
      </div>
      {children}
    </section>
  );
}
