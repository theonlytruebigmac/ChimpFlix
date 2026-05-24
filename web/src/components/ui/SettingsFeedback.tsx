"use client";

/// Shared success/error feedback used across the Settings pages so the
/// visual treatment lines up: emerald for success, muted red for
/// failures. Auto-clear timing (~2.5s) is the caller's responsibility.
///
/// Variants:
///   - `pill` (default): compact inline pill, sits next to a Save
///     button. Reserves a blank `&nbsp;` line when idle so layout
///     doesn't jump when the message appears/disappears.
///   - `block`: full-width callout, used as a page-level status row
///     (TwoFactor, Invites, etc.). Renders nothing when idle.
///
/// Either `message` or `error` may be set; pass `null` for the
/// unused half. The aria-live region stays mounted across changes so
/// screen readers announce updates without a remount cycle.
export function SettingsFeedback({
  message,
  error,
  variant = "pill",
  className,
}: {
  message?: string | null;
  error?: string | null;
  variant?: "pill" | "block";
  className?: string;
}) {
  if (variant === "block") {
    if (!message && !error) return null;
    if (error) {
      return (
        <div
          role="status"
          aria-live="polite"
          className={`rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300 ${className ?? ""}`}
        >
          {error}
        </div>
      );
    }
    return (
      <div
        role="status"
        aria-live="polite"
        className={`rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200 ${className ?? ""}`}
      >
        {message}
      </div>
    );
  }

  // pill variant
  if (!message && !error) {
    return (
      <span
        role="status"
        aria-live="polite"
        className={`text-xs text-transparent select-none ${className ?? ""}`}
      >
        {/* Reserve the line so the layout doesn't jump when feedback appears. */}
        &nbsp;
      </span>
    );
  }
  if (error) {
    return (
      <span
        role="status"
        aria-live="polite"
        className={`inline-flex items-center gap-1.5 rounded border border-red-500/30 bg-red-500/10 px-2 py-1 text-xs text-red-300 ${className ?? ""}`}
      >
        {error}
      </span>
    );
  }
  return (
    <span
      role="status"
      aria-live="polite"
      className={`inline-flex items-center gap-1.5 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200 ${className ?? ""}`}
    >
      <svg
        className="h-3 w-3"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
      {message}
    </span>
  );
}
