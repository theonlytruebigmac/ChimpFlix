"use client";

import { useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type User,
} from "@/lib/chimpflix-api";

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
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  async function save(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      const { user: updated } = await authApi.updateMe({
        display_name: displayName,
        avatar_url: avatarUrl,
        default_audio_lang: audioLang,
        default_subtitle_lang: subtitleLang,
      });
      setUser(updated);
      setMessage("Saved.");
      window.setTimeout(() => setMessage(null), 2500);
    } catch (e) {
      if (e instanceof ChimpFlixApiError) {
        setMessage(`Failed: HTTP ${e.status}`);
      } else {
        setMessage("Failed: network error");
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

      <div className="flex items-center gap-3 pt-1">
        <button
          type="submit"
          disabled={busy}
          className="rounded bg-(--color-accent) px-3 py-2 text-xs font-semibold text-white transition disabled:opacity-50"
        >
          {busy ? "Saving…" : "Save changes"}
        </button>
        {message && <span className="text-xs text-white/70">{message}</span>}
      </div>
    </form>
  );
}
