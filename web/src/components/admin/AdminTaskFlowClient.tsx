"use client";

/// Pipeline flow screen (Screen 4 from `docs/pipelines/tasks-ui.html`).
/// Renders the media-ingest pipeline as a clickable diagram: triggers
/// → scanner → FileAdded → Automatic/Gated fan-out. Each node carries
/// live in-flight + queued counts from the same `/overview` payload
/// that powers Screen 1, so no new endpoint is needed.

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type OverviewKindCard,
  type TasksOverviewResponse,
} from "@/lib/chimpflix-api";
import { ErrorBanner, Pill } from "./ui";

interface Props {
  initial: TasksOverviewResponse;
}

const REFRESH_MS = 5_000;

export function AdminTaskFlowClient({ initial }: Props) {
  const [overview, setOverview] = useState(initial);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setOverview(await adminApi.tasks.overview());
      setError(null);
    } catch (e) {
      setError(friendlyErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    const id = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(id);
  }, [refresh]);

  // Lookup by machine-stable id, not display strings. Renaming
  // "Media ingest pipeline" → "Ingestion" in tasks_overview.rs
  // won't silently empty the flow diagram.
  const mediaGroup = overview.groups.find((g) => g.id === "media_ingest");
  const automatic =
    mediaGroup?.sections.find((s) => s.id === "automatic")?.kinds ?? [];
  const gated =
    mediaGroup?.sections.find((s) => s.id === "gated")?.kinds ?? [];

  return (
    <div className="space-y-6">
      <ErrorBanner error={error} />

      <div className="flex items-center justify-end gap-2 text-xs text-white/60">
        <Link
          href="/settings/admin/tasks"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          ← Tasks
        </Link>
        <Link
          href="/settings/admin/tasks?tab=activity"
          className="rounded border border-white/15 px-2.5 py-1 transition-colors hover:bg-white/5"
        >
          Activity →
        </Link>
      </div>

      <Stage label="Triggers" desc="Three ways a library scan starts">
        <Row>
          <TriggerNode title="File watcher" sub="inotify · always on" tag="Daemon" tone="ok" />
          <TriggerNode title="Manual scan" sub="admin · per library" tag="Admin" tone="info" />
          <TriggerNode title="scan_library" sub="on_change cron" tag="Cron" tone="info" />
        </Row>
      </Stage>

      <Arrow />

      <Stage>
        <Row>
          <PlainNode title="Library scanner" sub="per-library lock · emits FileAdded" />
        </Row>
        <Arrow />
        <Row>
          <EventNode title="ScanEvent::FileAdded" sub="file_id · item_id · duration" />
        </Row>
      </Stage>

      <Arrow label="↓ fans out per file/item ↓" />

      <Group label="Discovery pipeline">
        <Subdivider label="Automatic" tone="info" />
        <Row wrap>
          {automatic.length === 0 ? (
            <EmptyNote text="No automatic kinds reported." />
          ) : (
            automatic.map((c) => <KindNode key={c.name} card={c} />)
          )}
        </Row>
        <Subdivider label="Gated · off by default" tone="warn" />
        <Row wrap>
          {gated.length === 0 ? (
            <EmptyNote text="No gated kinds reported." />
          ) : (
            gated.map((c) => <KindNode key={c.name} card={c} />)
          )}
        </Row>
      </Group>
    </div>
  );
}

// ─── Layout primitives ─────────────────────────────────────────────────

function Stage({
  label,
  desc,
  children,
}: {
  label?: string;
  desc?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-3">
      {label && (
        <div className="text-center">
          <h3 className="text-xs font-semibold uppercase tracking-[0.08em] text-white/55">
            {label}
          </h3>
          {desc && <p className="text-[11.5px] text-white/40">{desc}</p>}
        </div>
      )}
      {children}
    </section>
  );
}

function Row({ children, wrap = false }: { children: React.ReactNode; wrap?: boolean }) {
  return (
    <div
      className={`flex justify-center gap-3 ${wrap ? "flex-wrap" : "flex-wrap md:flex-nowrap"}`}
    >
      {children}
    </div>
  );
}

function Arrow({ label }: { label?: string }) {
  return (
    <div className="flex items-center justify-center gap-2 text-white/45">
      <span aria-hidden className="text-lg">
        ↓
      </span>
      {label && <span className="text-[11.5px] italic">{label}</span>}
    </div>
  );
}

function Group({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="relative rounded-2xl border border-dashed border-white/15 bg-white/3 px-4 py-6">
      <div className="absolute -top-3 left-6 rounded bg-(--color-background,#0d0d0d) px-2 text-[10.5px] font-semibold uppercase tracking-[0.08em] text-white/55">
        {label}
      </div>
      <div className="space-y-4">{children}</div>
    </div>
  );
}

function Subdivider({ label, tone }: { label: string; tone: "info" | "warn" }) {
  const cls =
    tone === "info" ? "text-blue-300/80 border-blue-300/20" : "text-amber-300/80 border-amber-300/20";
  return (
    <div className="flex items-center gap-3 px-1 text-[10.5px] font-semibold uppercase tracking-[0.08em]">
      <span className={`flex-1 border-t ${cls}`} />
      <span className={cls}>{label}</span>
      <span className={`flex-1 border-t ${cls}`} />
    </div>
  );
}

// ─── Node variants ─────────────────────────────────────────────────────

function TriggerNode({
  title,
  sub,
  tag,
  tone,
}: {
  title: string;
  sub: string;
  tag: string;
  tone: "ok" | "info" | "warn";
}) {
  const ring =
    tone === "ok"
      ? "border-emerald-400/40"
      : tone === "warn"
        ? "border-amber-400/40"
        : "border-blue-400/40";
  return (
    <div
      className={`relative w-44 rounded-xl border ${ring} bg-white/2 px-3 py-3 text-center shadow-sm`}
    >
      <span className="absolute -top-2 left-3 rounded bg-(--color-background,#0d0d0d) px-1.5 text-[9.5px] font-semibold uppercase tracking-[0.07em] text-white/55">
        {tag}
      </span>
      <div className="text-[12.5px] font-semibold text-white/95">{title}</div>
      <div className="mt-0.5 text-[11px] text-white/45">{sub}</div>
    </div>
  );
}

function PlainNode({ title, sub }: { title: string; sub: string }) {
  return (
    <div className="w-56 rounded-xl border border-white/10 bg-white/2 px-3 py-3 text-center">
      <div className="text-[12.5px] font-semibold text-white/95">{title}</div>
      <div className="mt-0.5 text-[11px] text-white/45">{sub}</div>
    </div>
  );
}

function EventNode({ title, sub }: { title: string; sub: string }) {
  return (
    <div className="w-56 rounded-xl border border-dashed border-white/15 bg-white/4 px-3 py-3 text-center font-mono">
      <div className="text-[12px] font-semibold text-white/90">{title}</div>
      <div className="mt-0.5 text-[11px] text-white/45">{sub}</div>
    </div>
  );
}

function KindNode({ card }: { card: OverviewKindCard }) {
  const href = `/settings/admin/tasks/kind/${encodeURIComponent(card.name)}`;
  const isGated = card.mode === "gated";
  const off = isGated && !card.gate.enabled;
  return (
    <Link
      href={href}
      className={`block w-48 rounded-xl border px-3 py-3 transition-colors hover:bg-white/5 ${
        off
          ? "border-white/10 bg-white/2 opacity-55"
          : isGated
            ? "border-amber-400/40 bg-white/3"
            : "border-blue-400/40 bg-white/3"
      }`}
    >
      <div className="flex items-start justify-between gap-2">
        <span className="text-[12.5px] font-semibold text-white/95">
          {card.display_name}
        </span>
        {card.live.in_flight > 0 ? (
          <Pill tone="info" dot>
            {card.live.in_flight}
          </Pill>
        ) : off ? (
          <Pill tone="muted">off</Pill>
        ) : (
          <Pill tone="muted">idle</Pill>
        )}
      </div>
      <div className="mt-1 font-mono text-[10.5px] text-white/40">
        {card.name}
      </div>
      <div className="mt-1 text-[11px] text-white/55">
        {card.live.queued > 0
          ? `${card.live.queued} queued`
          : card.live.last_success_at_ms
            ? "idle"
            : "never run"}
      </div>
    </Link>
  );
}

function EmptyNote({ text }: { text: string }) {
  return (
    <div className="rounded-xl border border-dashed border-white/10 bg-white/2 px-4 py-3 text-center text-[11.5px] text-white/45">
      {text}
    </div>
  );
}
