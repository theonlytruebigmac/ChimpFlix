/// Consistent header for every page under /settings/admin/*. Replaces
/// the bespoke `<header><h1><p>` block that each admin page used to
/// re-declare — pulled out so the Netflix-ish eyebrow + title spacing
/// can change in one place. The optional `eyebrow` slot lets a page
/// label its section (e.g. "Server", "Library") in addition to the
/// breadcrumb in the sidebar.
export function AdminPageHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  eyebrow?: string;
  title: string;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <header className="mb-8 flex flex-wrap items-end justify-between gap-4 border-b border-white/10 pb-5">
      <div className="min-w-0">
        {eyebrow && (
          <div className="mb-1 text-[0.7rem] font-semibold uppercase tracking-[0.18em] text-(--color-accent)">
            {eyebrow}
          </div>
        )}
        <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
        {description && (
          <p className="mt-1.5 max-w-2xl text-sm text-white/55">
            {description}
          </p>
        )}
      </div>
      {actions && <div className="shrink-0">{actions}</div>}
    </header>
  );
}
