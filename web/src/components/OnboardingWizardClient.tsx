"use client";

/// First-run wizard. Four steps: Welcome → Add a library → Paste a
/// TMDB read-token → Done. Reuses the existing admin endpoints
/// (`/libraries`, `/admin/secrets/tmdb`, `/admin/settings`) rather
/// than introducing a dedicated onboarding API surface — the
/// wizard is just an opinionated client over the same primitives
/// the regular admin pages use.
///
/// Flips `server_settings.setup_completed = true` on finish (and
/// also on Skip) so the auto-redirect from `/` only happens once.

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";

import {
  admin as adminApi,
  friendlyErrorMessage,
  libraries as librariesApi,
  type LibraryKind,
  type ScanJob,
  type ServerSettings,
} from "@/lib/chimpflix-api";

interface Props {
  initialSettings: ServerSettings;
}

type StepId = "welcome" | "library" | "tmdb" | "done";

const STEP_ORDER: StepId[] = ["welcome", "library", "tmdb", "done"];

const STEP_LABEL: Record<StepId, string> = {
  welcome: "Welcome",
  library: "Add a library",
  tmdb: "Metadata",
  done: "All set",
};

export function OnboardingWizardClient({ initialSettings }: Props) {
  const router = useRouter();
  const [step, setStep] = useState<StepId>("welcome");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Cross-step state — what the wizard has accomplished so far.
  // Surfaced on the Done step as a checklist so the operator
  // sees exactly what's set up before they land on Home.
  const [createdLibrary, setCreatedLibrary] = useState<{
    id: number;
    name: string;
  } | null>(null);
  const [tmdbSaved, setTmdbSaved] = useState(false);

  const next = STEP_ORDER[STEP_ORDER.indexOf(step) + 1] ?? "done";

  async function complete() {
    setBusy(true);
    setError(null);
    try {
      await adminApi.settings.patch({ setup_completed: true });
      router.push("/");
      router.refresh();
    } catch (e) {
      setError(friendlyErrorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mx-auto flex min-h-screen max-w-3xl flex-col px-6 py-10">
      <header className="mb-8">
        <div className="text-accent text-[0.7rem] font-semibold uppercase tracking-[0.18em]">
          {STEP_LABEL[step]}
        </div>
        <h1 className="mt-1 text-3xl font-bold tracking-tight text-white/95">
          {step === "welcome" && `Welcome, ${greeting(initialSettings)}`}
          {step === "library" && "Add your first library"}
          {step === "tmdb" && "Connect a metadata source"}
          {step === "done" && "You're ready to go"}
        </h1>
        <Progress current={step} />
      </header>

      {error && (
        <div className="mb-4 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-300">
          {error}
        </div>
      )}

      <main className="flex-1">
        {step === "welcome" && (
          <WelcomeStep
            onContinue={() => setStep("library")}
            onSkip={complete}
            busy={busy}
          />
        )}
        {step === "library" && (
          <LibraryStep
            onCreated={(library) => {
              setCreatedLibrary(library);
              setStep(next);
            }}
            onSkip={() => setStep(next)}
            onError={setError}
            busy={busy}
            setBusy={setBusy}
          />
        )}
        {step === "tmdb" && (
          <TmdbStep
            onSaved={() => {
              setTmdbSaved(true);
              setStep(next);
            }}
            onSkip={() => setStep(next)}
            onError={setError}
            busy={busy}
            setBusy={setBusy}
          />
        )}
        {step === "done" && (
          <DoneStep
            createdLibrary={createdLibrary}
            tmdbSaved={tmdbSaved}
            onFinish={complete}
            busy={busy}
          />
        )}
      </main>
    </div>
  );
}

// ─── Progress indicator ────────────────────────────────────────────────

function Progress({ current }: { current: StepId }) {
  const currentIdx = STEP_ORDER.indexOf(current);
  return (
    <ol className="mt-6 flex items-center gap-3 text-[12px]">
      {STEP_ORDER.map((s, i) => {
        const done = i < currentIdx;
        const active = i === currentIdx;
        return (
          <li key={s} className="flex items-center gap-3">
            <span
              className={`flex h-6 w-6 items-center justify-center rounded-full border text-[11px] font-semibold ${
                active
                  ? "border-accent bg-accent/15 text-accent"
                  : done
                    ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-300"
                    : "border-white/15 bg-white/4 text-white/45"
              }`}
            >
              {done ? "✓" : i + 1}
            </span>
            <span
              className={
                active
                  ? "font-medium text-white"
                  : done
                    ? "text-white/65"
                    : "text-white/45"
              }
            >
              {STEP_LABEL[s]}
            </span>
            {i < STEP_ORDER.length - 1 && (
              <span aria-hidden className="text-white/20">
                /
              </span>
            )}
          </li>
        );
      })}
    </ol>
  );
}

// ─── Welcome step ──────────────────────────────────────────────────────

function WelcomeStep({
  onContinue,
  onSkip,
  busy,
}: {
  onContinue: () => void;
  onSkip: () => void;
  busy: boolean;
}) {
  return (
    <div className="space-y-6">
      <p className="text-base text-white/75">
        ChimpFlix is your media server. In the next three steps we&apos;ll
        get you to your first watchable title:
      </p>
      <ul className="space-y-3">
        <Bullet
          title="Point at media on disk"
          body="Tell ChimpFlix which folders to watch. It scans automatically and keeps the library in sync as you add files."
        />
        <Bullet
          title="Plug in a metadata source"
          body="TMDB powers posters, descriptions, and recommendations. The free read-token tier is plenty for personal use."
        />
        <Bullet
          title="Start watching"
          body="Direct play when your client supports the codec, transcode when it doesn't. Hardware acceleration is auto-detected."
        />
      </ul>
      <div className="flex items-center justify-between pt-4">
        <button
          type="button"
          onClick={onSkip}
          disabled={busy}
          className="text-sm text-white/55 underline-offset-2 hover:text-white/85 hover:underline disabled:opacity-50"
        >
          Skip setup, I&apos;ll figure it out
        </button>
        <button
          type="button"
          onClick={onContinue}
          disabled={busy}
          className="rounded-md bg-accent px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-accent/85 disabled:opacity-50"
        >
          Let&apos;s go →
        </button>
      </div>
    </div>
  );
}

function Bullet({ title, body }: { title: string; body: string }) {
  return (
    <li className="rounded-lg border border-white/10 bg-white/2 px-4 py-3">
      <div className="text-[13.5px] font-semibold text-white/95">{title}</div>
      <div className="mt-0.5 text-[12.5px] text-white/55">{body}</div>
    </li>
  );
}

// ─── Add-library step ──────────────────────────────────────────────────

function LibraryStep({
  onCreated,
  onSkip,
  onError,
  busy,
  setBusy,
}: {
  onCreated: (library: { id: number; name: string }) => void;
  onSkip: () => void;
  onError: (msg: string | null) => void;
  busy: boolean;
  setBusy: (v: boolean) => void;
}) {
  const [name, setName] = useState("Movies");
  const [kind, setKind] = useState<LibraryKind>("movies");
  const [path, setPath] = useState("");

  async function submit() {
    onError(null);
    const trimmedName = name.trim();
    const trimmedPath = path.trim();
    if (!trimmedName) {
      onError("Give your library a name (e.g. 'Movies').");
      return;
    }
    if (!trimmedPath) {
      onError("Add at least one folder path the server can read.");
      return;
    }
    setBusy(true);
    try {
      // Duplicate guard. Re-running the wizard (e.g. after a
      // type-lie crash that left an empty library behind, or
      // intentionally from Server → General) used to silently
      // create a second row pointing at the same path — leaving
      // operators with two identically-named entries in the nav,
      // one empty, one populated. Check the existing set first; if
      // we find a row matching by path OR by name, adopt it
      // (re-trigger its scan) instead of inserting a duplicate.
      //
      // Path match wins over name match because the path is the
      // operationally significant identifier — two libraries
      // pointing at the same folder is always a misconfig, while
      // a name collision could be deliberate (two folders both
      // tagged "Anime"). Same-name + different-path is treated
      // as "use the existing one" since the wizard's whole purpose
      // is the one-library happy path.
      const { libraries: existing } = await librariesApi.list();
      const pathLower = trimmedPath.toLowerCase();
      const nameLower = trimmedName.toLowerCase();
      const dup =
        existing.find((l) =>
          l.paths.some((p) => p.trim().toLowerCase() === pathLower),
        ) ??
        existing.find((l) => l.name.trim().toLowerCase() === nameLower);

      const library = dup
        ? dup
        : await librariesApi.create({
            name: trimmedName,
            kind,
            paths: [trimmedPath],
          });

      // Kick off the scan immediately so files start landing while
      // the operator clicks through the rest of the wizard. Adopt
      // path: still trigger — re-scanning is idempotent and lets
      // the Done card show live progress even when reusing an
      // existing library row.
      try {
        await librariesApi.triggerScan(library.id);
      } catch (scanErr) {
        // Inline log but don't toast — the Done step's scan card
        // will reflect whatever's actually happening (in particular
        // it'll show "another scan already running" via the
        // listScans poll).
        console.warn("triggerScan failed during onboarding:", scanErr);
      }
      onCreated({ id: library.id, name: library.name });
    } catch (e) {
      onError(friendlyErrorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-5">
      <p className="text-sm text-white/65">
        Libraries group your media by type. Paths are read from inside the
        server container — if you mounted{" "}
        <code className="rounded bg-white/8 px-1.5 py-0.5 font-mono text-[12px] text-white/85">
          /mnt/archive
        </code>{" "}
        on the host as{" "}
        <code className="rounded bg-white/8 px-1.5 py-0.5 font-mono text-[12px] text-white/85">
          /media
        </code>{" "}
        in the container, use a path under{" "}
        <code className="rounded bg-white/8 px-1.5 py-0.5 font-mono text-[12px] text-white/85">
          /media
        </code>{" "}
        here.
      </p>

      <Field label="Library name">
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Movies"
          disabled={busy}
          className="w-full rounded-md border border-white/15 bg-black/30 px-3 py-2 text-sm text-white/90 focus:border-white/35 focus:outline-none"
        />
      </Field>

      <Field label="Library type">
        <div className="grid grid-cols-3 gap-2">
          {(
            [
              { v: "movies", label: "Movies", sub: "TMDB movies" },
              { v: "shows", label: "TV Shows", sub: "TMDB shows" },
              { v: "anime", label: "Anime", sub: "AniList + TMDB" },
            ] as const
          ).map((opt) => (
            <button
              key={opt.v}
              type="button"
              onClick={() => {
                setKind(opt.v);
                if (
                  name === "" ||
                  name === "Movies" ||
                  name === "TV Shows" ||
                  name === "Anime"
                ) {
                  setName(opt.label);
                }
              }}
              disabled={busy}
              className={`rounded-md border px-3 py-3 text-left transition-colors ${
                kind === opt.v
                  ? "border-accent bg-accent/10"
                  : "border-white/15 bg-white/2 hover:border-white/30"
              }`}
            >
              <div className="text-[13px] font-semibold text-white/95">
                {opt.label}
              </div>
              <div className="text-[11.5px] text-white/55">{opt.sub}</div>
            </button>
          ))}
        </div>
      </Field>

      <Field label="Media path">
        <input
          type="text"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="/media/movies"
          disabled={busy}
          className="w-full rounded-md border border-white/15 bg-black/30 px-3 py-2 font-mono text-[13px] text-white/90 focus:border-white/35 focus:outline-none"
        />
        <p className="mt-1 text-[11.5px] text-white/45">
          You can add more paths and per-library options later under Library →
          Libraries.
        </p>
      </Field>

      <div className="flex items-center justify-between pt-2">
        <button
          type="button"
          onClick={onSkip}
          disabled={busy}
          className="text-sm text-white/55 underline-offset-2 hover:text-white/85 hover:underline disabled:opacity-50"
        >
          Skip for now
        </button>
        <button
          type="button"
          onClick={submit}
          disabled={busy}
          className="rounded-md bg-accent px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-accent/85 disabled:opacity-50"
        >
          {busy ? "Creating…" : "Add library →"}
        </button>
      </div>
    </div>
  );
}

// ─── TMDB step ─────────────────────────────────────────────────────────

function TmdbStep({
  onSaved,
  onSkip,
  onError,
  busy,
  setBusy,
}: {
  onSaved: () => void;
  onSkip: () => void;
  onError: (msg: string | null) => void;
  busy: boolean;
  setBusy: (v: boolean) => void;
}) {
  const [token, setToken] = useState("");

  async function save() {
    onError(null);
    if (!token.trim()) {
      onError("Paste your TMDB read-token, or click Skip.");
      return;
    }
    setBusy(true);
    try {
      // PUT validates the token (the secrets handler tests TMDB on
      // save and 400s on a bad value), so a successful response
      // means the key works.
      await adminApi.secrets.set("tmdb", token.trim());
      onSaved();
    } catch (e) {
      onError(friendlyErrorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-5">
      <p className="text-sm text-white/65">
        TMDB provides posters, descriptions, cast, and recommendations for
        movies and TV shows. The{" "}
        <a
          href="https://www.themoviedb.org/settings/api"
          target="_blank"
          rel="noreferrer"
          className="text-white/85 underline underline-offset-2 hover:text-white"
        >
          read-only API token
        </a>{" "}
        is free — sign in, request a v4 token, paste it here.
      </p>

      <Field label="TMDB v4 read-token">
        <input
          // `type="text"` rather than "password" so the operator can
          // visually verify the paste (TMDB JWTs are long; getting a
          // wrong/truncated paste with no way to see it is the
          // common failure mode). `autoComplete="off"` keeps the
          // browser's password manager out of it.
          type="text"
          name="tmdb-token"
          autoComplete="off"
          spellCheck={false}
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="eyJhbGciOiJIUzI1NiJ9..."
          disabled={busy}
          className="w-full rounded-md border border-white/15 bg-black/30 px-3 py-2 font-mono text-[12px] text-white/90 focus:border-white/35 focus:outline-none"
        />
        <p className="mt-1 text-[11.5px] text-white/45">
          Stored encrypted in the credential vault. We test it on save and
          reject malformed tokens.
        </p>
      </Field>

      <div className="rounded-md border border-emerald-500/25 bg-emerald-500/5 px-3 py-2.5 text-[12.5px] text-emerald-200">
        ✓ AniList enrichment is already enabled for anime libraries — no API
        key required.
      </div>

      <div className="flex items-center justify-between pt-2">
        <button
          type="button"
          onClick={onSkip}
          disabled={busy}
          className="text-sm text-white/55 underline-offset-2 hover:text-white/85 hover:underline disabled:opacity-50"
        >
          Skip — I&apos;ll add it later
        </button>
        <button
          type="button"
          onClick={save}
          disabled={busy}
          className="rounded-md bg-accent px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-accent/85 disabled:opacity-50"
        >
          {busy ? "Verifying…" : "Save token →"}
        </button>
      </div>
    </div>
  );
}

// ─── Done step ─────────────────────────────────────────────────────────

function DoneStep({
  createdLibrary,
  tmdbSaved,
  onFinish,
  busy,
}: {
  createdLibrary: { id: number; name: string } | null;
  tmdbSaved: boolean;
  onFinish: () => void;
  busy: boolean;
}) {
  return (
    <div className="space-y-6">
      <p className="text-sm text-white/65">Here&apos;s what you set up:</p>
      <ul className="space-y-2">
        <Recap
          done={createdLibrary !== null}
          label={
            createdLibrary
              ? `Library "${createdLibrary.name}" created`
              : "Library — skipped"
          }
          tip={
            createdLibrary
              ? null
              : "Add one later under Library → Libraries."
          }
        />
        <Recap
          done={tmdbSaved}
          label={
            tmdbSaved
              ? "TMDB token saved — metadata will fill in as files are discovered"
              : "TMDB — skipped"
          }
          tip={
            tmdbSaved
              ? null
              : "Paste a token later under Server → Credentials."
          }
        />
        <Recap
          done
          label="AniList enrichment is on for anime"
          tip="No action needed — it's free and keyless."
        />
      </ul>

      {createdLibrary && (
        <ScanProgressCard
          libraryId={createdLibrary.id}
          libraryName={createdLibrary.name}
        />
      )}

      <div className="rounded-md border border-white/10 bg-white/2 px-4 py-3 text-[12.5px] text-white/65">
        <div className="font-semibold text-white/90">What&apos;s next</div>
        <ul className="mt-2 space-y-1">
          <li>
            • <strong>Hardware transcoding</strong> auto-detects on first
            playback. Tweak it under Server → Transcoder.
          </li>
          <li>
            • <strong>Scheduled tasks</strong> (intro detection, chapter
            thumbs, loudness) live under Library → Scheduled Tasks. Most are
            off by default — flip on what you want.
          </li>
          <li>
            • <strong>Invite others</strong> from Users → Invites once you
            have at least one library.
          </li>
        </ul>
      </div>

      <div className="flex items-center justify-end pt-2">
        <button
          type="button"
          onClick={onFinish}
          disabled={busy}
          className="rounded-md bg-accent px-5 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-accent/85 disabled:opacity-50"
        >
          {busy ? "Finishing…" : "Take me home →"}
        </button>
      </div>
    </div>
  );
}

/// Live scan progress for the just-created library. Polls
/// `/libraries/{id}/scans` every 1.5s and pulls the newest job —
/// shows file counters while running, a green check when done,
/// and a red badge with the error_message if the scan failed.
/// Mounts inside the Done step so the operator sees their library
/// fill up before they navigate to Home.
function ScanProgressCard({
  libraryId,
  libraryName,
}: {
  libraryId: number;
  libraryName: string;
}) {
  const [scan, setScan] = useState<ScanJob | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function tick() {
      try {
        const { scans } = await librariesApi.listScans(libraryId);
        if (cancelled) return;
        // Newest first — the API returns them ordered by created_at
        // desc, so [0] is the scan we just kicked off.
        setScan(scans[0] ?? null);
        setError(null);
      } catch (e) {
        if (cancelled) return;
        setError(friendlyErrorMessage(e));
      }
    }
    tick();
    const id = window.setInterval(tick, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [libraryId]);

  const status = scan?.status ?? "queued";
  const isRunning = status === "queued" || status === "running";
  const tone =
    status === "succeeded"
      ? "ok"
      : status === "failed"
        ? "bad"
        : "info";

  return (
    <div
      className={`overflow-hidden rounded-lg border ${
        tone === "ok"
          ? "border-emerald-500/30 bg-emerald-500/5"
          : tone === "bad"
            ? "border-red-500/30 bg-red-500/5"
            : "border-white/10 bg-white/2"
      }`}
    >
      <div className="flex items-center justify-between gap-3 border-b border-white/8 px-4 py-3">
        <div>
          <div className="text-[13px] font-semibold text-white/95">
            Scanning &ldquo;{libraryName}&rdquo;
          </div>
          <div className="text-[11.5px] text-white/55">
            {isRunning
              ? "Walking the directory tree and adding files to the queue. Metadata fetches happen as items finish."
              : status === "succeeded"
                ? "Initial pass finished. The file watcher keeps things in sync from here."
                : status === "failed"
                  ? "Scan failed — check the path and permissions, then try again from Library → Libraries."
                  : "Queued — waiting for an open scanner slot."}
          </div>
        </div>
        <ScanStatusPill status={status} />
      </div>
      <div className="grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-4">
        <ScanStat label="Seen" value={scan?.files_seen ?? 0} />
        <ScanStat
          label="Added"
          value={scan?.files_added ?? 0}
          tone="ok"
        />
        <ScanStat label="Updated" value={scan?.files_updated ?? 0} />
        <ScanStat
          label="Removed"
          value={scan?.files_removed ?? 0}
          tone={(scan?.files_removed ?? 0) > 0 ? "warn" : undefined}
        />
      </div>
      {error && (
        <div className="border-t border-white/8 bg-red-500/10 px-4 py-2 text-[11.5px] text-red-300">
          Couldn&apos;t fetch scan progress: {error}
        </div>
      )}
      {scan?.error_message && status === "failed" && (
        <div className="border-t border-white/8 bg-red-500/10 px-4 py-2 font-mono text-[11.5px] text-red-300">
          {scan.error_message}
        </div>
      )}
    </div>
  );
}

function ScanStatusPill({ status }: { status: ScanJob["status"] }) {
  const map: Record<ScanJob["status"], { label: string; cls: string }> = {
    queued: {
      label: "Queued",
      cls: "bg-white/8 text-white/65 ring-white/15",
    },
    running: {
      label: "Scanning…",
      cls: "bg-blue-500/15 text-blue-300 ring-blue-500/30",
    },
    succeeded: {
      label: "Done",
      cls: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30",
    },
    failed: {
      label: "Failed",
      cls: "bg-red-500/15 text-red-300 ring-red-500/30",
    },
    canceled: {
      label: "Canceled",
      cls: "bg-white/8 text-white/55 ring-white/15",
    },
  };
  const m = map[status];
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ${m.cls}`}
    >
      {status === "running" && (
        <span
          aria-hidden
          className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-blue-300"
        />
      )}
      {m.label}
    </span>
  );
}

function ScanStat({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: "ok" | "warn";
}) {
  const valueCls =
    tone === "ok"
      ? "text-emerald-300"
      : tone === "warn"
        ? "text-amber-300"
        : "text-white/90";
  return (
    <div>
      <div className="text-[10.5px] font-semibold uppercase tracking-[0.07em] text-white/45">
        {label}
      </div>
      <div className={`mt-0.5 text-xl font-semibold tabular-nums ${valueCls}`}>
        {value.toLocaleString()}
      </div>
    </div>
  );
}

function Recap({
  done,
  label,
  tip,
}: {
  done: boolean;
  label: string;
  tip: string | null;
}) {
  return (
    <li className="flex items-start gap-3 rounded-md border border-white/10 bg-white/2 px-3 py-2.5">
      <span
        className={`mt-0.5 inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[10px] font-bold ${
          done
            ? "bg-emerald-500/20 text-emerald-300 ring-1 ring-emerald-500/40"
            : "bg-white/10 text-white/50 ring-1 ring-white/15"
        }`}
      >
        {done ? "✓" : "—"}
      </span>
      <div className="min-w-0">
        <div className="text-[13px] text-white/90">{label}</div>
        {tip && (
          <div className="mt-0.5 text-[11.5px] text-white/45">{tip}</div>
        )}
      </div>
    </li>
  );
}

// ─── Shared field wrapper ──────────────────────────────────────────────

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1.5 block text-[12px] font-medium text-white/75">
        {label}
      </label>
      {children}
    </div>
  );
}

function greeting(settings: ServerSettings): string {
  // server_name defaults to "ChimpFlix"; show the operator's own
  // server name when they've already renamed it so the welcome line
  // reads "Welcome, MyServer". Otherwise fall back to a generic
  // line by passing the brand name.
  return settings.server_name || "ChimpFlix";
}
