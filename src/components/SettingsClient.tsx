"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { brandName } from "@/lib/env";
import { usePrefs } from "@/lib/prefs";
import type { Section } from "@/lib/plex-types";

export function SettingsClient({
  libraries,
  initialHidden,
}: {
  libraries: Section[];
  initialHidden: string[];
}) {
  const [prefs, update] = usePrefs();
  const router = useRouter();
  const [hidden, setHidden] = useState<Set<string>>(
    () => new Set(initialHidden),
  );
  const [saving, setSaving] = useState(false);

  async function toggleLibrary(key: string) {
    const next = new Set(hidden);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    setHidden(next);
    setSaving(true);
    try {
      await fetch("/api/prefs/libraries", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ hidden: [...next] }),
      });
      // Re-render server pages so the change takes effect on home/movies/shows
      // without a hard reload.
      router.refresh();
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="px-12 pb-24 pt-28">
      <h1 className="mb-2 text-4xl font-bold tracking-tight">Settings</h1>
      <p className="mb-10 max-w-xl text-sm text-white/60">
        Stored in this browser. Sign-in / Plex account settings live on
        plex.tv.
      </p>

      <section className="max-w-xl space-y-1">
        <h2 className="mb-3 text-xs font-semibold uppercase tracking-wider text-white/50">
          Playback
        </h2>

        <ToggleRow
          label="Autoplay next episode"
          description="When an episode ends, automatically start the next one. A 10-second countdown card shows before the jump so you can cancel."
          value={prefs.autoplayNext}
          onChange={(v) => update({ autoplayNext: v })}
        />

        <div className="border-t border-white/10 pt-4">
          <ReadOnlyRow
            label="Default volume"
            value={`${Math.round(prefs.volume * 100)}%`}
          />
          <ReadOnlyRow
            label="Default playback speed"
            value={prefs.playbackRate === 1 ? "Normal" : `${prefs.playbackRate}×`}
          />
          <p className="mt-2 text-xs text-white/50">
            Volume and speed update automatically as you adjust them in the
            player.
          </p>
        </div>
      </section>

      <section className="mt-14 max-w-xl">
        <h2 className="mb-3 text-xs font-semibold uppercase tracking-wider text-white/50">
          Libraries
          {saving && (
            <span className="ml-2 text-white/40 normal-case tracking-normal">
              · saving…
            </span>
          )}
        </h2>
        <p className="mb-4 text-sm text-white/60">
          Hide libraries you don&apos;t want surfaced in the home rails or the
          Movies / Shows pages. The library still exists on your Plex server
          — this just controls what shows up in {brandName()}.
        </p>

        {libraries.length === 0 ? (
          <p className="text-sm text-white/50">
            No libraries returned from Plex.
          </p>
        ) : (
          <ul className="divide-y divide-white/10">
            {libraries.map((lib) => {
              const isHidden = hidden.has(lib.key);
              return (
                <li
                  key={lib.key}
                  className="flex items-center justify-between gap-6 py-4"
                >
                  <div>
                    <div className="text-base font-medium">{lib.title}</div>
                    <div className="mt-0.5 text-xs uppercase tracking-wide text-white/50">
                      {lib.type}
                    </div>
                  </div>
                  <ToggleSwitch
                    value={!isHidden}
                    onChange={() => toggleLibrary(lib.key)}
                    ariaLabel={`Show ${lib.title}`}
                  />
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}

function ToggleRow({
  label,
  description,
  value,
  onChange,
}: {
  label: string;
  description: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-6 py-4">
      <div className="flex-1">
        <div className="text-base font-medium">{label}</div>
        <div className="mt-1 text-sm text-white/60">{description}</div>
      </div>
      <ToggleSwitch
        value={value}
        onChange={() => onChange(!value)}
        ariaLabel={label}
      />
    </div>
  );
}

function ToggleSwitch({
  value,
  onChange,
  ariaLabel,
}: {
  value: boolean;
  onChange: () => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={value}
      aria-label={ariaLabel}
      onClick={onChange}
      className={`relative mt-1 inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200 ${
        value ? "bg-(--color-accent)" : "bg-white/25"
      }`}
    >
      <span
        aria-hidden
        className={`inline-block h-5 w-5 rounded-full bg-white shadow ring-0 transition-transform duration-200 ${
          value ? "translate-x-5" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}

function ReadOnlyRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between py-3 text-sm">
      <span className="text-white/70">{label}</span>
      <span className="tabular-nums text-white">{value}</span>
    </div>
  );
}
