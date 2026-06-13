"use client";

import { useEffect, useRef, useState } from "react";
import {
  auth as authApi,
  ChimpFlixApiError,
  type User,
} from "@/lib/chimpflix-api";

interface Props {
  initial: User;
  /// Drives the "Owner" pill in the card head. Read-only context.
  isOwner?: boolean;
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

export function SettingsProfileClient({ initial, isOwner = false }: Props) {
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
  // Reset to true when the avatar URL changes so a freshly-typed, broken
  // URL re-attempts a load instead of staying stuck on the monogram.
  const [avatarFailed, setAvatarFailed] = useState(false);

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

  const monogram = (displayName.trim() || user.username || "?")
    .charAt(0)
    .toUpperCase();
  const showAvatarImg = avatarUrl.trim().length > 0 && !avatarFailed;

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

  function discard() {
    setDisplayName(user.display_name ?? "");
    setAvatarUrl(user.avatar_url ?? "");
    setAudioLang(user.default_audio_lang ?? "");
    setSubtitleLang(user.default_subtitle_lang ?? "");
    setNotifyEmail(user.notify_via_email);
    setError(null);
  }

  return (
    <form onSubmit={save}>
      {/* ── Profile card ──────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Profile</div>
            <div className="cf-sub">How you appear across ChimpFlix.</div>
          </div>
          {isOwner && (
            <div className="cf-head-aside">
              <span className="cf-pill cf-ok">
                <span className="cf-dot" />
                Owner
              </span>
            </div>
          )}
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Display name</div>
              <div className="cf-row-help">
                Shown on activity and in the admin console. Defaults to your
                username.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder={user.username}
                maxLength={64}
                className={
                  "cf-input cf-w-auto" +
                  (displayName !== (user.display_name ?? "") ? " cf-changed" : "")
                }
                style={{ minWidth: 240 }}
              />
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Avatar</div>
              <div className="cf-row-help">
                A direct image URL. Leave blank for the auto monogram.
              </div>
            </div>
            <div className="cf-row-control">
              {showAvatarImg ? (
                // eslint-disable-next-line @next/next/no-img-element
                <img
                  src={avatarUrl}
                  alt="Avatar preview"
                  width={38}
                  height={38}
                  onError={() => setAvatarFailed(true)}
                  style={{
                    width: 38,
                    height: 38,
                    borderRadius: "50%",
                    objectFit: "cover",
                    flex: "none",
                  }}
                />
              ) : (
                <span
                  className="cf-avatar cf-a1"
                  style={{ width: 38, height: 38 }}
                  aria-hidden
                >
                  {monogram}
                </span>
              )}
              <input
                type="url"
                value={avatarUrl}
                onChange={(e) => {
                  setAvatarUrl(e.target.value);
                  setAvatarFailed(false);
                }}
                placeholder="https://…/me.jpg"
                className={
                  "cf-input cf-w-auto" +
                  (avatarUrl !== (user.avatar_url ?? "") ? " cf-changed" : "")
                }
                style={{ minWidth: 240 }}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Playback defaults card ────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Playback defaults</div>
            <div className="cf-sub">
              Auto-selected when a title has matching tracks; falls back
              gracefully otherwise.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Default audio language</div>
            </div>
            <div className="cf-row-control">
              <select
                value={audioLang}
                onChange={(e) => setAudioLang(e.target.value)}
                className={
                  "cf-select cf-w-auto" +
                  (audioLang !== (user.default_audio_lang ?? "")
                    ? " cf-changed"
                    : "")
                }
              >
                {LANG_OPTIONS.map((o) => (
                  <option key={o.code || "_none"} value={o.code}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Default subtitles</div>
            </div>
            <div className="cf-row-control">
              <select
                value={subtitleLang}
                onChange={(e) => setSubtitleLang(e.target.value)}
                className={
                  "cf-select cf-w-auto" +
                  (subtitleLang !== (user.default_subtitle_lang ?? "")
                    ? " cf-changed"
                    : "")
                }
              >
                {LANG_OPTIONS.map((o) => (
                  <option key={o.code || "_none"} value={o.code}>
                    {o.label === "— No preference —" ? "— Off —" : o.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">
                Email me when I get a notification
              </div>
              <div className="cf-row-help">
                Mirrors the in-app bell to your email. Needs an email on file and
                SMTP configured by the admin.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={notifyEmail}
                aria-label="Email me when I get a notification"
                disabled={!user.email}
                className={"cf-switch" + (notifyEmail ? " cf-on" : "")}
                onClick={() => setNotifyEmail((v) => !v)}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── sticky save bar — only while there are unsaved changes ── */}
      {(hasChanges || error || message) && (
        <div className="cf-savebar">
          <div className="cf-sb-status">
            {error ? (
              <>
                <span className="cf-dot" style={{ background: "var(--err)" }} />
                <b style={{ color: "#fff" }}>{error}</b>
              </>
            ) : hasChanges ? (
              <>
                <span className="cf-dot" style={{ background: "var(--warn)" }} />
                <b style={{ color: "#fff" }}>Unsaved changes</b>
              </>
            ) : (
              <>
                <span className="cf-dot" style={{ background: "var(--ok)" }} />
                {message}
              </>
            )}
          </div>
          <div className="cf-sb-actions">
            <button
              type="button"
              className="cf-btn cf-ghost cf-sm"
              onClick={discard}
              disabled={busy || !hasChanges}
            >
              Discard
            </button>
            <button
              type="submit"
              className="cf-btn cf-primary cf-sm"
              disabled={busy || !hasChanges}
            >
              {busy ? "Saving…" : "Save changes"}
            </button>
          </div>
        </div>
      )}
    </form>
  );
}
