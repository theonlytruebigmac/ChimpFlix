"use client";

/// Shared placeholder for sections waiting on their initial fetch.
/// Replaces ad-hoc `<p>Loading…</p>` strings sprinkled across admin
/// and settings clients so the visual treatment is consistent and
/// the spinner gives the reader a real "yes, this is still working"
/// signal vs reading as broken-page text.
///
/// Variants:
///   - `block` (default): centered spinner + label, sized for a
///     section/page-level load.
///   - `inline`: small inline spinner + label, sized to sit next to
///     a heading or live in a narrow row.
export function LoadingPlaceholder({
  label = "Loading…",
  variant = "block",
  className,
}: {
  label?: string;
  variant?: "block" | "inline";
  className?: string;
}) {
  if (variant === "inline") {
    return (
      <span
        role="status"
        aria-live="polite"
        className={`inline-flex items-center gap-1.5 text-xs text-white/55 ${className ?? ""}`}
      >
        <Spinner size={3} />
        {label}
      </span>
    );
  }
  return (
    <div
      role="status"
      aria-live="polite"
      className={`flex items-center justify-center gap-2 py-6 text-sm text-white/55 ${className ?? ""}`}
    >
      <Spinner size={4} />
      <span>{label}</span>
    </div>
  );
}

function Spinner({ size }: { size: 3 | 4 }) {
  const cls = size === 3 ? "h-3 w-3" : "h-4 w-4";
  return (
    <svg
      className={`${cls} animate-spin`}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden
    >
      <circle
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeOpacity="0.25"
        strokeWidth="3"
      />
      <path
        d="M22 12a10 10 0 0 0-10-10"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
      />
    </svg>
  );
}
