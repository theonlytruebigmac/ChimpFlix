"use client";

/// Shared lightweight password-strength hint. Used under every
/// new-password field (signup, reset, account change). We *don't* gate
/// submit on the score — the backend enforces a length floor and
/// there's a real usability cost to refusing pass-phrases that look
/// "weak" by dictionary heuristics. The hint exists to nudge, not to
/// gate.
export function PasswordStrengthHint({ value }: { value: string }) {
  if (!value) {
    return (
      <span className="mt-1 block text-xs text-white/40">
        At least 8 characters. Mix in numbers or symbols for a stronger one.
      </span>
    );
  }
  const score = scorePassword(value);
  const label =
    score === 0
      ? "Too short"
      : score === 1
        ? "Weak"
        : score === 2
          ? "Okay"
          : score === 3
            ? "Strong"
            : "Very strong";
  const colour =
    score <= 1
      ? "bg-red-400/80"
      : score === 2
        ? "bg-amber-400/80"
        : "bg-emerald-400/80";
  const textColour =
    score <= 1
      ? "text-red-300"
      : score === 2
        ? "text-amber-200"
        : "text-emerald-200";
  return (
    <div className="mt-1.5">
      <div className="flex h-1 w-full overflow-hidden rounded bg-white/5">
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            className={`flex-1 ${i < score ? colour : "bg-transparent"} ${i > 0 ? "ml-0.5" : ""}`}
          />
        ))}
      </div>
      <span
        className={`mt-1 block text-xs ${textColour}`}
        aria-live="polite"
      >
        {label}
      </span>
    </div>
  );
}

/// 0-4 strength bucket. Cheap heuristic — length is the dominant
/// signal, with small bumps for character-class diversity. Anything
/// fancier (zxcvbn, dictionary lookups) would dwarf the rest of the
/// auth bundle for no real security gain since the backend enforces
/// its own floor.
export function scorePassword(value: string): number {
  if (value.length < 8) return 0;
  let s = 1;
  if (value.length >= 12) s += 1;
  if (value.length >= 16) s += 1;
  const classes =
    Number(/[a-z]/.test(value)) +
    Number(/[A-Z]/.test(value)) +
    Number(/\d/.test(value)) +
    Number(/[^A-Za-z0-9]/.test(value));
  if (classes >= 3) s += 1;
  return Math.min(s, 4);
}
