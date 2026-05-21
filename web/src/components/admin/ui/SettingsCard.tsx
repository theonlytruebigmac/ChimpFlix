import type { ReactNode } from "react";

/// Settings-form card. Owns a header (title + optional description) and
/// a stack of `SettingsRow` children. Every admin settings page is a
/// vertical stack of these, so the spacing/border/heading typography
/// only lives in one place. See `AdminTranscoderClient` for the
/// canonical usage pattern.
export function SettingsCard({
  title,
  description,
  aside,
  flat,
  children,
}: {
  title: string;
  description?: ReactNode;
  /// Right-aligned slot in the header for a status pill, "Beta" badge,
  /// or single action that scopes to the card (e.g. "Detect" for a
  /// hardware probe). Buttons that act on the form as a whole belong
  /// in the SaveBar, not here.
  aside?: ReactNode;
  /// Drop the outer border + page-style padding. Use when nesting
  /// SettingsCards inside another container that already has its own
  /// chrome — e.g. inside a Drawer body where the drawer's border is
  /// the visual frame and a card border would double up.
  flat?: boolean;
  children: ReactNode;
}) {
  if (flat) {
    return (
      <section className="mb-4 last:mb-0">
        <header className="flex items-baseline justify-between gap-3 border-b border-white/6 pb-2 mb-2">
          <div className="min-w-0">
            <h3 className="m-0 text-[12.5px] font-semibold">{title}</h3>
            {description && (
              <p className="mt-0.5 text-[11.5px] text-white/55 leading-relaxed">
                {description}
              </p>
            )}
          </div>
          {aside && <div className="shrink-0">{aside}</div>}
        </header>
        <div>{children}</div>
      </section>
    );
  }
  return (
    <section className="mb-4 overflow-hidden rounded-lg border border-white/10 bg-white/2">
      <header className="flex items-baseline justify-between gap-4 border-b border-white/10 px-5 py-4">
        <div className="min-w-0">
          <h3 className="m-0 text-sm font-semibold">{title}</h3>
          {description && (
            <p className="mt-0.5 text-xs text-white/55 leading-relaxed">
              {description}
            </p>
          )}
        </div>
        {aside && <div className="shrink-0">{aside}</div>}
      </header>
      <div>{children}</div>
    </section>
  );
}

/// Two-column form row: label + help on the left, control on the
/// right. The label column is fixed-width so a stack of rows reads as
/// a clean grid even when help text wraps. `changed` adds a subtle
/// amber outline so reviewers can scan a long form and spot
/// modifications at a glance.
export function SettingsRow({
  label,
  help,
  changed,
  stacked,
  flat,
  children,
}: {
  label: ReactNode;
  help?: ReactNode;
  /// Set true when the row's control value differs from its baseline.
  /// Used purely as a visual hint — the actual dirty-state lives in
  /// the parent form, and the SaveBar reads the count from there.
  changed?: boolean;
  /// Force a single-column layout (label on top, control below) at
  /// every viewport. Use inside narrow containers like Drawer where
  /// the default 280-px label column squeezes the control to nothing.
  stacked?: boolean;
  /// Drop the row's outer padding + tighten the border. Pair with the
  /// `flat` SettingsCard inside a Drawer body so rows feel like a
  /// continuous stack instead of a card-within-a-card.
  flat?: boolean;
  children: ReactNode;
}) {
  const padding = flat ? "py-3" : "px-5 py-4";
  const border = flat
    ? "border-b border-white/6 last:border-b-0"
    : "border-b border-white/10 last:border-b-0";
  const cols = stacked
    ? "grid-cols-1 gap-2"
    : "grid-cols-1 gap-3 md:grid-cols-[280px_1fr] md:gap-6";
  return (
    <div
      className={`grid ${cols} ${border} ${padding} ${changed ? "bg-amber-500/2.5" : ""}`}
    >
      <div className="min-w-0">
        <div className="text-[13px] font-medium leading-snug">{label}</div>
        {help && (
          <div className="mt-1 text-[11.5px] text-white/55 leading-relaxed">
            {help}
          </div>
        )}
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}
