"use client";

/// Pipeline flow screen (Screen 4 from `docs/pipelines/tasks-ui.html`),
/// styled in the console design language (`cf-*`) to match the
/// redesign mockup: a horizontal fan-out diagram inside one card —
/// trigger → classify/match/enrich → detect-markers/subtitles/… —
/// with live in-flight + queued counts on each node. Each kind node
/// is a clickable drill-in. Powered by the same `/overview` payload
/// that drives the Overview tab, so no new endpoint is needed.

import { useCallback, useEffect, useState, type CSSProperties } from "react";
import Link from "next/link";

import {
  admin as adminApi,
  friendlyErrorMessage,
  type OverviewKindCard,
  type TasksOverviewResponse,
} from "@/lib/chimpflix-api";

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
  // "Media ingest pipeline" → "Ingestion" in tasks_overview.rs won't
  // silently empty the flow diagram.
  const mediaGroup = overview.groups.find((g) => g.id === "media_ingest");
  const automatic =
    mediaGroup?.sections.find((s) => s.id === "automatic")?.kinds ?? [];
  const gated =
    mediaGroup?.sections.find((s) => s.id === "gated")?.kinds ?? [];

  return (
    <div>
      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      <p className="cf-muted" style={{ fontSize: 13, marginTop: 0 }}>
        When a media file is added, work fans out through these stages. Live
        counts update in place — click any node to drill into that kind.
      </p>

      <div className="cf-card" style={{ marginBottom: 0 }}>
        <div className="cf-card-body cf-pad" style={{ overflowX: "auto" }}>
          <div style={{ display: "flex", gap: 18, alignItems: "stretch", minWidth: 760 }}>
            <FlowCol>
              <TriggerNode />
            </FlowCol>
            <FlowArrow />
            <FlowCol>
              {automatic.length === 0 ? (
                <EmptyNode text="No automatic kinds reported." />
              ) : (
                automatic.map((c) => <KindNode key={c.name} card={c} />)
              )}
            </FlowCol>
            {gated.length > 0 && (
              <>
                <FlowArrow />
                <FlowCol>
                  {gated.map((c) => (
                    <KindNode key={c.name} card={c} />
                  ))}
                </FlowCol>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Layout primitives ─────────────────────────────────────────────────

function FlowCol({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 12,
        justifyContent: "center",
      }}
    >
      {children}
    </div>
  );
}

function FlowArrow() {
  return (
    <div
      aria-hidden
      style={{
        display: "grid",
        placeItems: "center",
        color: "var(--ghost)",
        fontSize: 22,
      }}
    >
      →
    </div>
  );
}

// ─── Node variants ─────────────────────────────────────────────────────

const NODE_BASE: CSSProperties = {
  border: "1px solid var(--line)",
  borderRadius: "var(--r)",
  background: "var(--surface)",
  padding: "11px 13px",
  minWidth: 170,
  display: "block",
};

function nodeDot(color: string): CSSProperties {
  return {
    width: 7,
    height: 7,
    borderRadius: "50%",
    background: color,
    display: "inline-block",
  };
}

const NODE_HEAD: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  fontWeight: 600,
  fontSize: 13,
};
const NODE_COUNTS: CSSProperties = {
  marginTop: 7,
  display: "flex",
  gap: 10,
  fontSize: 11,
  color: "var(--muted)",
};

function TriggerNode() {
  return (
    <div style={NODE_BASE}>
      <div style={NODE_HEAD}>
        <svg
          style={{ width: 15, height: 15 }}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <path d="M12 5v14M5 12h14" />
        </svg>
        Media file added
      </div>
      <div style={NODE_COUNTS}>
        <span>trigger</span>
      </div>
    </div>
  );
}

function KindNode({ card }: { card: OverviewKindCard }) {
  const href = `/settings/admin/tasks/kind/${encodeURIComponent(card.name)}`;
  const isGated = card.mode === "gated";
  const off = isGated && !card.gate.enabled;
  const failing = card.schedule?.last_status === "bad";
  const running = card.live.in_flight > 0;

  // Node ring + dot color reflect live state, mirroring the mockup's
  // run / fail / idle node treatments.
  const style: CSSProperties = { ...NODE_BASE };
  let dotColor = "var(--ghost)";
  if (failing) {
    style.borderColor = "rgba(248,113,113,.35)";
    dotColor = "var(--err)";
  } else if (running) {
    style.borderColor = "var(--info-soft)";
    style.boxShadow = "0 0 0 1px rgba(96,165,250,.25)";
    dotColor = "var(--info)";
  } else if (off) {
    style.opacity = 0.55;
  } else if (card.live.queued === 0 && card.live.last_success_at_ms) {
    dotColor = "var(--ok)";
  }

  return (
    <Link href={href} style={style} aria-label={`Open detail for ${card.display_name}`}>
      <div style={NODE_HEAD}>
        <span style={nodeDot(dotColor)} />
        {card.display_name}
      </div>
      <div style={NODE_COUNTS}>
        <span>
          <b style={{ color: "#fff" }}>{card.live.queued}</b> queued
        </span>
        {failing ? (
          <span style={{ color: "#fca5a5" }}>failing</span>
        ) : running ? (
          <span>
            <b style={{ color: "#fff" }}>{card.live.in_flight}</b> running
          </span>
        ) : off ? (
          <span>off</span>
        ) : card.live.last_success_at_ms ? (
          <span>idle</span>
        ) : (
          <span>never run</span>
        )}
      </div>
    </Link>
  );
}

function EmptyNode({ text }: { text: string }) {
  return (
    <div style={{ ...NODE_BASE, borderStyle: "dashed", color: "var(--faint)" }}>
      <div style={{ fontSize: 12 }}>{text}</div>
    </div>
  );
}
