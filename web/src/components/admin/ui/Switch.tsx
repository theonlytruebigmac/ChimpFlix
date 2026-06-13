"use client";

/// On/off toggle, styled to the Netflix-dark system. Accessible
/// `button[role="switch"]` — pair with `SettingsRow` (label + help on the
/// left, this on the right). Controlled: the parent owns the boolean and
/// reflects dirty-state into the SaveBar, same as every other form control.
export function Switch({
  checked,
  onChange,
  disabled = false,
  id,
  "aria-label": ariaLabel,
  className = "",
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  id?: string;
  "aria-label"?: string;
  className?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      id={id}
      aria-label={ariaLabel}
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-6 w-[42px] shrink-0 items-center rounded-full border transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        checked
          ? "border-(--color-accent) bg-(--color-accent)"
          : "border-white/15 bg-white/15"
      } ${className}`}
    >
      <span
        aria-hidden
        className={`absolute left-0.5 top-1/2 inline-block h-[18px] w-[18px] -translate-y-1/2 rounded-full bg-white shadow transition-transform ${
          checked ? "translate-x-[18px]" : ""
        }`}
      />
    </button>
  );
}
