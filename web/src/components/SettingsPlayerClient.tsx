"use client";

import { usePrefs } from "@/lib/prefs";

// Device-local player prefs. These live in localStorage rather than on the
// user record because they're per-browser ergonomics — what feels right on
// a TV-connected laptop differs from what feels right on a phone.
export function SettingsPlayerClient() {
  const [prefs, updatePrefs] = usePrefs();

  return (
    <div className="space-y-3">
      <p className="text-xs text-white/55">
        Saved on this device only. Tab between browsers and these reset to
        defaults.
      </p>

      <Toggle
        label="Autoplay next episode"
        description="Roll into the next episode of a series automatically."
        checked={prefs.autoplayNext}
        onChange={(v) => updatePrefs({ autoplayNext: v })}
      />

      <Toggle
        label="Mute hero trailers"
        description="Preview clips on the home page start without sound."
        checked={prefs.trailerMuted}
        onChange={(v) => updatePrefs({ trailerMuted: v })}
      />
    </div>
  );
}

function Toggle({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start justify-between gap-4 rounded border border-white/5 bg-white/5 px-4 py-3 text-sm transition-colors hover:bg-white/10">
      <span>
        <span className="block font-medium">{label}</span>
        <span className="mt-0.5 block text-xs text-white/55">
          {description}
        </span>
      </span>
      <span className="relative inline-flex h-6 w-11 shrink-0 items-center">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className="peer sr-only"
        />
        <span className="absolute inset-0 rounded-full bg-white/15 transition-colors peer-checked:bg-(--color-accent)" />
        <span className="absolute left-0.5 inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform peer-checked:translate-x-5" />
      </span>
    </label>
  );
}
