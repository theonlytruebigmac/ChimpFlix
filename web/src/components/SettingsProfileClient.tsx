"use client";

import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type User,
} from "@/lib/chimpflix-api";
import { SettingsFeedback } from "./ui/SettingsFeedback";

interface Props {
  initial: User;
}

const LANG_OPTIONS: ReadonlyArray<{ code: string; label: string }> = [
  { code: "", label: "— No preference —" },
  { code: "eng", label: "English" },
  { code: "spa", label: "Spanish" },
  { code: "fra", label: "French" },
  { code: "deu", label: "German" },
  { code: "ita", label: "Italian" },
  { code: "por", label: "Portuguese" },
  { code: "jpn", label: "Japanese" },
  { code: "kor", label: "Korean" },
  { code: "chi", label: "Chinese" },
  { code: "rus", label: "Russian" },
  { code: "ara", label: "Arabic" },
  { code: "hin", label: "Hindi" },
  { code: "nld", label: "Dutch" },
];

export function SettingsProfileClient({ initial }: Props) {
  const [user, setUser] = useState(initial);
  const [displayName, setDisplayName] = useState(initial.display_name ?? "");
  const [avatarUrl, setAvatarUrl] = useState(initial.avatar_url ?? "");
  const [audioLang, setAudioLang] = useState(initial.default_audio_lang ?? "");
  const [subtitleLang, setSubtitleLang] = useState(
    initial.default_subtitle_lang ?? "",
  );
  const [notifyEmail, setNotifyEmail] = useState(initial.notify_via_email);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Compare against the most-recent server state (`user`, not the
  // immutable `initial` prop) so the Save button correctly disables
  // again right after a successful save — `user` is what the next save
  // would actually no-op against.
  const hasChanges =
    displayName !== (user.display_name ?? "") ||
    avatarUrl !== (user.avatar_url ?? "") ||
    audioLang !== (user.default_audio_lang ?? "") ||
    subtitleLang !== (user.default_subtitle_lang ?? "") ||
    notifyEmail !== user.notify_via_email;
  // Holds the auto-clear timer so we can cancel it on unmount and
  // avoid setState-on-unmounted warnings (and a small leak) if the
  // user navigates away inside the 2.5s window.
  const messageTimerRef = useRef<number | null>(null);
  useEffect(() => {
    return () => {
      if (messageTimerRef.current !== null) {
        window.clearTimeout(messageTimerRef.current);
        messageTimerRef.current = null;
      }
    };
  }, []);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setMessage(null);
    setError(null);
    try {
      const { user: updated } = await authApi.updateMe({
        display_name: displayName,
        avatar_url: avatarUrl,
        default_audio_lang: audioLang,
        default_subtitle_lang: subtitleLang,
        notify_via_email: notifyEmail,
      });
      setUser(updated);
      setMessage("Saved");
      if (messageTimerRef.current !== null) {
        window.clearTimeout(messageTimerRef.current);
      }
      messageTimerRef.current = window.setTimeout(() => {
        messageTimerRef.current = null;
        setMessage(null);
      }, 2500);
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        // Surface the server's validation message (e.g. "email is
        // already in use by another account") instead of bare HTTP.
        let detail = `HTTP ${e.status}`;
        try {
          const parsed = JSON.parse(e.body) as {
            error?: { message?: string };
          };
          if (parsed.error?.message) detail = parsed.error.message;
        } catch {}
        setError(`Couldn't save: ${detail}`);
      } else {
        setError("Couldn't save. Network error.");
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={save} className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <label className="block text-xs">
          <span className="mb-1 block text-white/60">Display name</span>
          <input
            type="text"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            placeholder={user.username}
            maxLength={64}
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
        </label>
        <label className="block text-xs">
          <span className="mb-1 block text-white/60">
            Avatar URL <span className="text-white/40">(optional)</span>
          </span>
          <input
            type="url"
            value={avatarUrl}
            onChange={(e) => setAvatarUrl(e.target.value)}
            placeholder="https://…/me.jpg"
            className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
          />
        </label>
      </div>

      <div className="border-t border-white/10 pt-4">
        <h3 className="mb-3 text-sm font-semibold">Playback defaults</h3>
        <p className="mb-3 text-xs text-white/55">
          Auto-selects matching tracks on play. Picking a language that
          isn&apos;t available on a title just falls back to the default.
        </p>
        <div className="grid gap-4 sm:grid-cols-2">
          <label className="block text-xs">
            <span className="mb-1 block text-white/60">Default audio</span>
            <select
              value={audioLang}
              onChange={(e) => setAudioLang(e.target.value)}
              className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            >
              {LANG_OPTIONS.map((o) => (
                <option key={o.code || "_none"} value={o.code}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          <label className="block text-xs">
            <span className="mb-1 block text-white/60">Default subtitles</span>
            <select
              value={subtitleLang}
              onChange={(e) => setSubtitleLang(e.target.value)}
              className="w-full rounded bg-white/10 px-3 py-2 text-sm outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
            >
              {LANG_OPTIONS.map((o) => (
                <option key={o.code || "_none"} value={o.code}>
                  {o.label === "— No preference —" ? "— Off —" : o.label}
                </option>
              ))}
            </select>
          </label>
        </div>
      </div>

      <div className="border-t border-white/10 pt-4">
        <h3 className="mb-3 text-sm font-semibold">Notifications</h3>
        <label className="flex items-start gap-3 text-xs">
          <input
            type="checkbox"
            checked={notifyEmail}
            onChange={(e) => setNotifyEmail(e.target.checked)}
            disabled={!user.email}
            className="mt-1"
          />
          <div>
            <div className="text-white">Email me when I get a notification</div>
            <div className="mt-0.5 text-white/50">
              Mirrors the in-app bell to your email. Requires an email on
              your profile and SMTP configured by the admin.
            </div>
          </div>
        </label>
      </div>

      <div className="flex items-center gap-3 pt-1">
        <button
          type="submit"
          disabled={busy || !hasChanges}
          className="rounded bg-(--color-accent) px-4 py-2.5 text-sm font-semibold text-white sm:px-3 sm:py-2 sm:text-xs transition disabled:opacity-50"
        >
          {busy ? "Saving…" : "Save changes"}
        </button>
        <SettingsFeedback message={message} error={error} />
      </div>
    </form>
  );
}
