"use client";

import { useMemo, useState } from "react";
import { auth as authApi, type User } from "@/lib/chimpflix-api";

// ─────────────────────────────────────────────────────────────────────────
// Notification kinds the user can tune.
//
// Wire discriminators MUST match the backend `KIND_*` constants in
// crates/server/src/notifier.rs:
//   content.new_episode  → KIND_NEW_EPISODE  (new content, per-user)
//   content.new_movie    → KIND_NEW_MOVIE    (new content, per-user)
//   job.failed           → KIND_JOB_FAILED       (owner-only)
//   user.registered      → KIND_USER_REGISTERED  (owner-only)
//
// Security kinds (user.2fa.disabled / user.2fa.reset) are intentionally
// ABSENT from this list — they always deliver and can't be muted. They're
// surfaced as the uncloseable "Account & security alerts — Required" row.
// ─────────────────────────────────────────────────────────────────────────

interface KindPref {
  enabled: boolean;
  email: boolean;
}

interface KindDef {
  id: string;
  label: string;
  help: string;
  /** Owner-only kinds render only for role === "owner". */
  ownerOnly?: boolean;
}

// Viewer/content topics — visible to everyone. Backed by features 1+2.
const CONTENT_KINDS: KindDef[] = [
  {
    id: "content.new_episode",
    label: "New episodes in shows I watch",
    help: "When a new episode lands in a series on your Continue Watching row.",
  },
  {
    id: "content.new_movie",
    label: "New movies in my libraries",
    help: "Fresh additions to the libraries you can see.",
  },
];

// Owner-only admin topics — kept from the previous build; rendered only when
// the page receives an owner.
const OWNER_KINDS: KindDef[] = [
  {
    id: "job.failed",
    label: "Job failures",
    help: "A background job exhausted its retries (owner alert).",
    ownerOnly: true,
  },
  {
    id: "user.registered",
    label: "New user signups",
    help: "Someone accepted an invite and finished signup (owner alert).",
    ownerOnly: true,
  },
];

type ParsedKindPref = Partial<KindPref> & {
  quiet_start?: string;
  quiet_end?: string;
  // Legacy integer-hour bounds (pre-HH:MM). Read for back-compat.
  quiet_start_hour?: number;
  quiet_end_hour?: number;
};

function parsePrefs(raw: string): Record<string, ParsedKindPref> {
  try {
    const v = JSON.parse(raw || "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

const HHMM = /^([01]?\d|2[0-3]):([0-5]\d)$/;

/// Resolve one quiet-hours bound to an "HH:MM" string for the <input
/// type="time"> control. Prefers the new HH:MM key; falls back to the
/// legacy integer hour; otherwise the supplied default.
function quietBoundToHHMM(
  p: ParsedKindPref | undefined,
  hhmmKey: "quiet_start" | "quiet_end",
  hourKey: "quiet_start_hour" | "quiet_end_hour",
  fallback: string,
): string {
  const s = p?.[hhmmKey];
  if (typeof s === "string" && HHMM.test(s.trim())) {
    const [h, m] = s.trim().split(":");
    return `${h.padStart(2, "0")}:${m}`;
  }
  const h = p?.[hourKey];
  if (typeof h === "number" && h >= 0 && h <= 23) {
    return `${String(Math.round(h)).padStart(2, "0")}:00`;
  }
  return fallback;
}

/// The set of IANA zones for the timezone picker. Uses the browser's
/// canonical list when available (every evergreen browser supports it);
/// otherwise a small hand-picked fallback so the control always works.
function tzOptions(current: string): string[] {
  let zones: string[] = [];
  try {
    const supported = (
      Intl as unknown as { supportedValuesOf?: (k: string) => string[] }
    ).supportedValuesOf;
    if (typeof supported === "function") zones = supported("timeZone");
  } catch {
    zones = [];
  }
  if (zones.length === 0) {
    zones = [
      "UTC",
      "America/New_York",
      "America/Chicago",
      "America/Denver",
      "America/Los_Angeles",
      "Europe/London",
      "Europe/Paris",
      "Europe/Berlin",
      "Asia/Tokyo",
      "Asia/Shanghai",
      "Australia/Sydney",
    ];
  }
  // Always include UTC + whatever the user currently has stored.
  if (!zones.includes("UTC")) zones = ["UTC", ...zones];
  if (current && !zones.includes(current)) zones = [current, ...zones];
  return zones;
}

export function SettingsNotificationsClient({
  initial,
  isOwner,
}: {
  initial: User;
  isOwner: boolean;
}) {
  const parsed = useMemo(
    () => parsePrefs(initial.notification_prefs_json),
    [initial.notification_prefs_json],
  );

  // The kinds visible to THIS user — content always, owner kinds only for
  // owners. This list also drives what we persist.
  const visibleKinds = useMemo(
    () => (isOwner ? [...CONTENT_KINDS, ...OWNER_KINDS] : CONTENT_KINDS),
    [isOwner],
  );

  const [kinds, setKinds] = useState<Record<string, KindPref>>(() => {
    const out: Record<string, KindPref> = {};
    for (const k of [...CONTENT_KINDS, ...OWNER_KINDS]) {
      const p = parsed[k.id] ?? {};
      out[k.id] = { enabled: p.enabled ?? true, email: p.email ?? true };
    }
    return out;
  });

  // Master email channel — the same `notify_via_email` flag set on the
  // Profile page. Per-topic email toggles are gated on this.
  const [notifyEmail, setNotifyEmail] = useState<boolean>(initial.notify_via_email);
  const hasEmailOnFile = !!initial.email;

  // Personal Discord webhook — only when configured does the Discord column
  // appear (matches the notifier: Discord delivery requires a webhook).
  const hasDiscord = !!initial.discord_webhook_url;

  // One quiet-hours window, applied to every configurable kind. Seed from
  // whichever kind currently carries it.
  const seededQuiet = [...CONTENT_KINDS, ...OWNER_KINDS]
    .map((k) => parsed[k.id])
    .find(
      (p) =>
        p &&
        (typeof p.quiet_start === "string" ||
          typeof p.quiet_start_hour === "number"),
    );
  const [quietOn, setQuietOn] = useState<boolean>(!!seededQuiet);
  const [quietStart, setQuietStart] = useState<string>(
    quietBoundToHHMM(seededQuiet, "quiet_start", "quiet_start_hour", "22:00"),
  );
  const [quietEnd, setQuietEnd] = useState<string>(
    quietBoundToHHMM(seededQuiet, "quiet_end", "quiet_end_hour", "07:00"),
  );

  const [timezone, setTimezone] = useState<string>(initial.timezone || "UTC");
  const zones = useMemo(() => tzOptions(initial.timezone || "UTC"), [initial.timezone]);

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
    for (const k of visibleKinds) {
      const kp: Record<string, unknown> = {
        enabled: kinds[k.id].enabled,
        email: kinds[k.id].email,
      };
      if (quietOn) {
        kp.quiet_start = quietStart;
        kp.quiet_end = quietEnd;
      }
      prefs[k.id] = kp;
    }
    try {
      await authApi.updateMe({
        notification_prefs_json: JSON.stringify(prefs),
        notify_via_email: notifyEmail,
        timezone,
      });
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    } catch (e) {
      setError(
        e instanceof Error ? e.message : "Couldn't save notification preferences.",
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      {/* ── Delivery: where notifications show up ───────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Delivery</div>
            <div className="cf-sub">Where notifications show up.</div>
          </div>
          <div className="cf-head-aside">
            <span className="cf-pill">
              <BellIcon />
              {hasDiscord ? "3 channels" : "2 channels"}
            </span>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">In-app bell</div>
              <div className="cf-row-help">
                Always on — the bell in the top nav.
              </div>
            </div>
            <div className="cf-row-control">
              <span className="cf-pill cf-ok">
                <span className="cf-dot" />
                Always on
              </span>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Email</div>
              <div className="cf-row-help">
                {hasEmailOnFile ? (
                  <>
                    Mirror notifications to <b>{initial.email}</b>. Needs SMTP
                    configured by the admin.
                  </>
                ) : (
                  <>
                    Add an email on{" "}
                    <a href="/settings/account" className="cf-mono">
                      Account → Profile
                    </a>{" "}
                    to mirror notifications by email.
                  </>
                )}
              </div>
            </div>
            <div className="cf-row-control">
              <Switch
                label="Email mirroring"
                checked={notifyEmail && hasEmailOnFile}
                disabled={!hasEmailOnFile}
                onChange={setNotifyEmail}
              />
            </div>
          </div>
          {hasDiscord && (
            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-label">Discord</div>
                <div className="cf-row-help">
                  Mirror to your personal Discord webhook. Configured on{" "}
                  <a href="/settings/account" className="cf-mono">
                    Account
                  </a>
                  .
                </div>
              </div>
              <div className="cf-row-control">
                <span className="cf-pill cf-info">
                  <span className="cf-dot" />
                  Connected
                </span>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* ── Topics: what to notify me about ─────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">What to notify me about</div>
            <div className="cf-sub">
              Pick the events worth a ping. Account and security alerts
              can&rsquo;t be turned off.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          {visibleKinds.map((k) => (
            <TopicRow
              key={k.id}
              label={k.label}
              help={k.help}
              enabled={kinds[k.id].enabled}
              email={kinds[k.id].email}
              emailChannelOn={notifyEmail && hasEmailOnFile}
              onToggleEnabled={(v) => setKind(k.id, { enabled: v })}
              onToggleEmail={(v) => setKind(k.id, { email: v })}
            />
          ))}

          {/* Coming-soon rows from the mockup — NOT backed by a real kind.
              Rendered as honest disabled rows so we don't fake delivery. */}
          <ComingSoonRow
            label="Someone shares a playlist with me"
            help="When another profile shares a playlist to yours. Coming soon."
          />
          <ComingSoonRow
            label="Watch-party invites"
            help="Invitations to sync-watch a title with other profiles. Coming soon."
          />

          {/* Account & security — always on, can't be muted. Backed by the
              user.2fa.* security kinds (sign-ins / password / 2FA events). */}
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Account and security alerts</div>
              <div className="cf-row-help">
                Two-factor changes and other security events. Always delivered.
              </div>
            </div>
            <div className="cf-row-control">
              <span className="cf-pill">
                <LockIcon />
                Required
              </span>
            </div>
          </div>
        </div>
      </div>

      {/* ── Quiet hours ─────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Quiet hours</div>
            <div className="cf-sub">
              Hold back non-urgent pings overnight. Security alerts always come
              through.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">
                Pause non-urgent notifications overnight
              </div>
              <div className="cf-row-help">
                Email and Discord are suppressed during this window. The in-app
                bell still records so nothing is lost.
              </div>
            </div>
            <div className="cf-row-control">
              <Switch
                label="Quiet hours"
                checked={quietOn}
                onChange={setQuietOn}
              />
            </div>
          </div>
          {quietOn && (
            <>
              <div className="cf-row">
                <div className="cf-row-main">
                  <div className="cf-row-label">Window</div>
                  <div className="cf-row-help">Local to your time zone.</div>
                </div>
                <div className="cf-row-control cf-gap8">
                  <input
                    className="cf-input cf-w-auto"
                    type="time"
                    style={{ minWidth: 120 }}
                    aria-label="Quiet hours start"
                    value={quietStart}
                    onChange={(e) => setQuietStart(e.target.value)}
                  />
                  <span className="cf-faint">to</span>
                  <input
                    className="cf-input cf-w-auto"
                    type="time"
                    style={{ minWidth: 120 }}
                    aria-label="Quiet hours end"
                    value={quietEnd}
                    onChange={(e) => setQuietEnd(e.target.value)}
                  />
                </div>
              </div>
              <div className="cf-row">
                <div className="cf-row-main">
                  <div className="cf-row-label">Time zone</div>
                  <div className="cf-row-help">
                    Quiet hours are evaluated against this wall clock.
                  </div>
                </div>
                <div className="cf-row-control">
                  <select
                    className="cf-select cf-w-auto"
                    aria-label="Time zone"
                    value={timezone}
                    onChange={(e) => setTimezone(e.target.value)}
                  >
                    {zones.map((z) => (
                      <option key={z} value={z}>
                        {z}
                      </option>
                    ))}
                  </select>
                </div>
              </div>
            </>
          )}
        </div>
      </div>

      {/* ── Save bar ────────────────────────────────────────────────── */}
      <div className="cf-savebar">
        <div className="cf-sb-status">
          {error ? (
            <>
              <span className="cf-dot" style={{ background: "var(--err)" }} />
              {error}
            </>
          ) : saved ? (
            <>
              <span className="cf-dot" style={{ background: "var(--ok)" }} />
              All changes saved
            </>
          ) : (
            <>
              <span className="cf-dot" style={{ background: "var(--faint)" }} />
              Unsaved changes
            </>
          )}
        </div>
        <div className="cf-sb-actions">
          <button
            type="button"
            className="cf-btn cf-primary"
            onClick={save}
            disabled={saving}
          >
            {saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      </div>
    </div>
  );
}

/// A topic row with an enable toggle plus an email channel toggle. The email
/// toggle is disabled when the kind is off or the master email channel is off.
function TopicRow({
  label,
  help,
  enabled,
  email,
  emailChannelOn,
  onToggleEnabled,
  onToggleEmail,
}: {
  label: string;
  help: string;
  enabled: boolean;
  email: boolean;
  emailChannelOn: boolean;
  onToggleEnabled: (v: boolean) => void;
  onToggleEmail: (v: boolean) => void;
}) {
  return (
    <div className="cf-row">
      <div className="cf-row-main">
        <div className="cf-row-label">{label}</div>
        <div className="cf-row-help">{help}</div>
      </div>
      <div className="cf-row-control cf-gap16">
        <label
          className="cf-flex cf-gap8"
          style={{ fontSize: 12, color: "var(--muted)" }}
        >
          Email
          <Switch
            label={`${label} — email`}
            checked={email && enabled && emailChannelOn}
            disabled={!enabled || !emailChannelOn}
            onChange={onToggleEmail}
          />
        </label>
        <Switch
          label={label}
          checked={enabled}
          onChange={onToggleEnabled}
        />
      </div>
    </div>
  );
}

/// Honest "coming soon" row — the feature isn't built, so the control is a
/// disabled switch + a pill rather than a fake working toggle.
function ComingSoonRow({ label, help }: { label: string; help: string }) {
  return (
    <div className="cf-row">
      <div className="cf-row-main">
        <div className="cf-row-label" style={{ color: "var(--muted)" }}>
          {label}
        </div>
        <div className="cf-row-help">{help}</div>
      </div>
      <div className="cf-row-control">
        <span className="cf-pill">Coming soon</span>
      </div>
    </div>
  );
}

/// cf-* toggle switch (button role="switch"). Mirrors the mockup's `.switch`.
function Switch({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={`cf-switch${checked ? " cf-on" : ""}`}
      onClick={() => onChange(!checked)}
    />
  );
}

function BellIcon() {
  return (
    <svg
      style={{ width: 13, height: 13 }}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M6 9a6 6 0 0 1 12 0c0 7 2 8 2 8H4s2-1 2-8" />
      <path d="M10 21a2 2 0 0 0 4 0" />
    </svg>
  );
}

function LockIcon() {
  return (
    <svg
      style={{ width: 13, height: 13 }}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="5" y="11" width="14" height="9" rx="2" />
      <path d="M8 11V8a4 4 0 0 1 8 0v3" />
    </svg>
  );
}
