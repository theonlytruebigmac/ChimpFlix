"use client";

import { useMemo, useState } from "react";
import { auth as authApi, type User } from "@/lib/chimpflix-api";
import { Switch } from "@/components/admin/ui";

interface KindPref {
  enabled: boolean;
  email: boolean;
}

// The notification kinds a user can tune. Security kinds (user.2fa.*) are
// intentionally absent — they always deliver and can't be muted. Content
// kinds (new episode / new movie) will join this list when those per-user
// events ship; the framework already honors whatever kinds are present.
const KINDS: { id: string; label: string; help: string }[] = [
  {
    id: "job.failed",
    label: "Job failures",
    help: "A background job exhausted its retries (owner alert).",
  },
  {
    id: "user.registered",
    label: "New user signups",
    help: "Someone accepted an invite and finished signup (owner alert).",
  },
];

function parsePrefs(raw: string): Record<string, Partial<KindPref> & { quiet_start_hour?: number; quiet_end_hour?: number }> {
  try {
    const v = JSON.parse(raw || "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

const clampHour = (n: number) => Math.max(0, Math.min(23, Math.round(n) || 0));

export function SettingsNotificationsClient({ initial }: { initial: User }) {
  const parsed = useMemo(
    () => parsePrefs(initial.notification_prefs_json),
    [initial.notification_prefs_json],
  );

  const [kinds, setKinds] = useState<Record<string, KindPref>>(() => {
    const out: Record<string, KindPref> = {};
    for (const k of KINDS) {
      const p = parsed[k.id] ?? {};
      out[k.id] = { enabled: p.enabled ?? true, email: p.email ?? true };
    }
    return out;
  });

  // One quiet-hours window, applied to every configurable kind. Seed from
  // whichever kind currently carries it.
  const seededQuiet = KINDS.map((k) => parsed[k.id]).find(
    (p) => p && typeof p.quiet_start_hour === "number",
  );
  const [quietOn, setQuietOn] = useState<boolean>(!!seededQuiet);
  const [quietStart, setQuietStart] = useState<number>(seededQuiet?.quiet_start_hour ?? 22);
  const [quietEnd, setQuietEnd] = useState<number>(seededQuiet?.quiet_end_hour ?? 7);

  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const setKind = (id: string, patch: Partial<KindPref>) =>
    setKinds((prev) => ({ ...prev, [id]: { ...prev[id], ...patch } }));

  const save = async () => {
    setSaving(true);
    setError(null);
    setSaved(false);
    const prefs: Record<string, unknown> = {};
    for (const k of KINDS) {
      const kp: Record<string, unknown> = {
        enabled: kinds[k.id].enabled,
        email: kinds[k.id].email,
      };
      if (quietOn) {
        kp.quiet_start_hour = quietStart;
        kp.quiet_end_hour = quietEnd;
      }
      prefs[k.id] = kp;
    }
    try {
      await authApi.updateMe({ notification_prefs_json: JSON.stringify(prefs) });
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't save notification preferences.");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-7">
      <p className="max-w-2xl text-sm text-white/55">
        Choose which alerts reach you and when. Email also requires the master toggle in{" "}
        <a href="/settings/account" className="text-white underline">
          Account&nbsp;→&nbsp;Profile
        </a>{" "}
        and SMTP configured by your admin. Security alerts (two-factor changes) are always
        delivered and can&rsquo;t be muted.
      </p>

      <div className="divide-y divide-white/10">
        {KINDS.map((k) => (
          <div key={k.id} className="flex items-start justify-between gap-4 py-4">
            <div className="min-w-0">
              <div className="text-sm font-medium">{k.label}</div>
              <div className="mt-1 text-[12.5px] leading-relaxed text-white/55">{k.help}</div>
            </div>
            <div className="flex shrink-0 items-center gap-5">
              <label className="flex items-center gap-2 text-xs text-white/60">
                Email
                <Switch
                  checked={kinds[k.id].email}
                  onChange={(v) => setKind(k.id, { email: v })}
                  disabled={!kinds[k.id].enabled}
                />
              </label>
              <Switch
                checked={kinds[k.id].enabled}
                onChange={(v) => setKind(k.id, { enabled: v })}
                aria-label={`${k.label} enabled`}
              />
            </div>
          </div>
        ))}

        <div className="py-4">
          <div className="flex items-start justify-between gap-4">
            <div>
              <div className="text-sm font-medium">Quiet hours</div>
              <div className="mt-1 text-[12.5px] text-white/55">
                Hold back notification emails during this window. Bell alerts still record.
              </div>
            </div>
            <Switch checked={quietOn} onChange={setQuietOn} aria-label="Quiet hours" />
          </div>
          {quietOn && (
            <div className="mt-3 flex items-center gap-2 text-sm">
              <input
                type="number"
                min={0}
                max={23}
                value={quietStart}
                onChange={(e) => setQuietStart(clampHour(Number(e.target.value)))}
                className="w-16 rounded-md border border-white/15 bg-white/4 px-2 py-1.5 text-center outline-none focus:border-white/30"
              />
              <span className="text-white/45">to</span>
              <input
                type="number"
                min={0}
                max={23}
                value={quietEnd}
                onChange={(e) => setQuietEnd(clampHour(Number(e.target.value)))}
                className="w-16 rounded-md border border-white/15 bg-white/4 px-2 py-1.5 text-center outline-none focus:border-white/30"
              />
              <span className="text-xs text-white/45">hour of day, UTC</span>
            </div>
          )}
        </div>
      </div>

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={save}
          disabled={saving}
          className="rounded-md border border-(--color-accent) bg-(--color-accent) px-3 py-1.5 text-[13px] font-medium text-white transition-colors hover:bg-(--color-accent-hover) disabled:cursor-not-allowed disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
        {saved && (
          <span className="flex items-center gap-1.5 text-sm text-emerald-300">
            <span className="inline-block h-2 w-2 rounded-full bg-emerald-400" aria-hidden />
            Saved
          </span>
        )}
        {error && <span className="text-sm text-red-300">{error}</span>}
      </div>
    </div>
  );
}
