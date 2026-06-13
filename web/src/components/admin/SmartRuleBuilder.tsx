"use client";

import { useEffect, useState } from "react";

/// Field/op vocabulary mirrors the backend `smart_rule.rs` whitelist.
/// Keep these in sync — out-of-vocab values are silently rejected
/// server-side, so a UI/backend drift produces "nothing matches" with
/// no clear error.
const FIELD_DEFS = {
  kind: {
    label: "Type",
    ops: ["eq", "ne"] as const,
    valueKind: "enum" as const,
    enumValues: ["movie", "show", "episode"],
  },
  year: {
    label: "Year",
    ops: ["eq", "ne", "lt", "le", "gt", "ge", "between"] as const,
    valueKind: "int" as const,
  },
  rating_audience: {
    label: "Audience rating",
    ops: ["eq", "ne", "lt", "le", "gt", "ge", "between"] as const,
    valueKind: "real" as const,
  },
  library_id: {
    label: "Library",
    ops: ["eq", "ne", "in"] as const,
    valueKind: "int" as const,
  },
  title: {
    label: "Title",
    ops: ["eq", "contains", "starts_with"] as const,
    valueKind: "text" as const,
  },
  genre: {
    label: "Genre",
    ops: ["contains"] as const,
    valueKind: "text" as const,
  },
  tag: {
    label: "Tag",
    ops: ["contains"] as const,
    valueKind: "text" as const,
  },
  added_at: {
    label: "Added at (epoch ms — file acquisition time)",
    ops: ["lt", "le", "gt", "ge", "between"] as const,
    valueKind: "int" as const,
  },
} as const;

type FieldKey = keyof typeof FIELD_DEFS;
type OpKey = (typeof FIELD_DEFS)[FieldKey]["ops"][number];

const OP_LABELS: Record<OpKey, string> = {
  eq: "equals",
  ne: "not equals",
  lt: "less than",
  le: "less or equal",
  gt: "greater than",
  ge: "greater or equal",
  between: "between (inclusive)",
  in: "in set",
  contains: "contains",
  starts_with: "starts with",
};

interface UICondition {
  /// Stable key assigned at creation so React can reconcile correctly
  /// when conditions are removed or reordered.
  id: string;
  field: FieldKey;
  op: OpKey;
  /// Stored as string for the input field; coerced at serialize time.
  /// For `between` ops, two values; for `in`, comma-separated list;
  /// otherwise a single value.
  value: string;
  value2?: string;
}

interface UIRule {
  operator: "and" | "or";
  conditions: UICondition[];
}

interface Props {
  /// Initial rule JSON. Empty / invalid initial values reset to a
  /// single empty condition so the form is always usable.
  initialJson: string;
  /// Called on every change with the serialised rule JSON. Parent
  /// owns the actual save dispatch.
  onChange: (ruleJson: string) => void;
}

/// Structured field/op/value picker for smart collection rules.
/// Replaces the JSON textarea — operators don't need to know the wire
/// shape. Backend still re-validates so a future builder bug can't
/// store a broken rule.
export function SmartRuleBuilder({ initialJson, onChange }: Props) {
  const [rule, setRule] = useState<UIRule>(() => parseRule(initialJson));

  useEffect(() => {
    onChange(serialize(rule));
  }, [rule, onChange]);

  function updateCondition(idx: number, patch: Partial<UICondition>) {
    setRule((r) => ({
      ...r,
      conditions: r.conditions.map((c, i) =>
        i === idx ? { ...c, ...patch } : c,
      ),
    }));
  }

  function addCondition() {
    setRule((r) => ({
      ...r,
      conditions: [
        ...r.conditions,
        { id: crypto.randomUUID(), field: "kind" as FieldKey, op: "eq" as OpKey, value: "movie" },
      ],
    }));
  }

  function removeCondition(idx: number) {
    setRule((r) => ({
      ...r,
      conditions: r.conditions.length > 1
        ? r.conditions.filter((_, i) => i !== idx)
        : r.conditions,
    }));
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2 text-xs">
        <span className="text-white/55">Match</span>
        <select
          value={rule.operator}
          onChange={(e) =>
            setRule((r) => ({
              ...r,
              operator: e.target.value as "and" | "or",
            }))
          }
          className="rounded border border-white/10 bg-black/30 px-2 py-1"
        >
          <option value="and">ALL of the following (AND)</option>
          <option value="or">ANY of the following (OR)</option>
        </select>
      </div>
      <ul className="space-y-2">
        {rule.conditions.map((c, idx) => (
          <li
            key={c.id}
            className="flex flex-wrap items-center gap-2 rounded border border-white/10 bg-black/30 p-2"
          >
            <ConditionRow
              cond={c}
              onChange={(patch) => updateCondition(idx, patch)}
              onRemove={() => removeCondition(idx)}
              canRemove={rule.conditions.length > 1}
            />
          </li>
        ))}
      </ul>
      <button
        type="button"
        onClick={addCondition}
        className="rounded border border-white/15 px-2 py-1 text-xs text-white/80 hover:bg-white/5"
      >
        + Add condition
      </button>
    </div>
  );
}

function ConditionRow({
  cond,
  onChange,
  onRemove,
  canRemove,
}: {
  cond: UICondition;
  onChange: (patch: Partial<UICondition>) => void;
  onRemove: () => void;
  canRemove: boolean;
}) {
  const def = FIELD_DEFS[cond.field];
  const allowedOps = def.ops as readonly OpKey[];
  const opOk = allowedOps.includes(cond.op);
  const op = opOk ? cond.op : allowedOps[0];

  return (
    <>
      <select
        value={cond.field}
        onChange={(e) => {
          const nextField = e.target.value as FieldKey;
          const nextOps = FIELD_DEFS[nextField].ops as readonly OpKey[];
          onChange({
            field: nextField,
            op: nextOps[0],
            value: "",
            value2: undefined,
          });
        }}
        className="rounded border border-white/10 bg-black/40 px-2 py-1 text-xs"
      >
        {(Object.keys(FIELD_DEFS) as FieldKey[]).map((k) => (
          <option key={k} value={k}>
            {FIELD_DEFS[k].label}
          </option>
        ))}
      </select>
      <select
        value={op}
        onChange={(e) => onChange({ op: e.target.value as OpKey })}
        className="rounded border border-white/10 bg-black/40 px-2 py-1 text-xs"
      >
        {allowedOps.map((o) => (
          <option key={o} value={o}>
            {OP_LABELS[o]}
          </option>
        ))}
      </select>
      <ValueInput
        cond={{ ...cond, op }}
        onChange={onChange}
      />
      <button
        type="button"
        onClick={onRemove}
        disabled={!canRemove}
        className="ml-auto rounded border border-red-500/40 px-2 py-1 text-xs text-red-300 hover:bg-red-500/10 disabled:cursor-not-allowed disabled:opacity-30"
      >
        ✕
      </button>
    </>
  );
}

function ValueInput({
  cond,
  onChange,
}: {
  cond: UICondition;
  onChange: (patch: Partial<UICondition>) => void;
}) {
  const def = FIELD_DEFS[cond.field];

  if (cond.op === "between") {
    return (
      <>
        <input
          type={def.valueKind === "text" ? "text" : "number"}
          value={cond.value}
          onChange={(e) => onChange({ value: e.target.value })}
          placeholder="from"
          className="w-24 rounded border border-white/10 bg-black/40 px-2 py-1 text-xs"
        />
        <span className="text-xs text-white/55">and</span>
        <input
          type={def.valueKind === "text" ? "text" : "number"}
          value={cond.value2 ?? ""}
          onChange={(e) => onChange({ value2: e.target.value })}
          placeholder="to"
          className="w-24 rounded border border-white/10 bg-black/40 px-2 py-1 text-xs"
        />
      </>
    );
  }

  if (cond.op === "in") {
    return (
      <input
        type="text"
        value={cond.value}
        onChange={(e) => onChange({ value: e.target.value })}
        placeholder="comma-separated"
        className="w-48 rounded border border-white/10 bg-black/40 px-2 py-1 text-xs font-mono"
      />
    );
  }

  if (def.valueKind === "enum" && "enumValues" in def) {
    return (
      <select
        value={cond.value || def.enumValues[0]}
        onChange={(e) => onChange({ value: e.target.value })}
        className="rounded border border-white/10 bg-black/40 px-2 py-1 text-xs"
      >
        {def.enumValues.map((v) => (
          <option key={v} value={v}>
            {v}
          </option>
        ))}
      </select>
    );
  }

  return (
    <input
      type={def.valueKind === "text" ? "text" : "number"}
      value={cond.value}
      onChange={(e) => onChange({ value: e.target.value })}
      placeholder="value"
      className="w-32 rounded border border-white/10 bg-black/40 px-2 py-1 text-xs"
    />
  );
}

// ─── parse / serialize ────────────────────────────────────────────────────

function parseRule(json: string): UIRule {
  const fallback: UIRule = {
    operator: "and",
    conditions: [{ id: crypto.randomUUID(), field: "kind", op: "eq", value: "movie" }],
  };
  if (!json.trim()) return fallback;
  try {
    const parsed = JSON.parse(json) as unknown;
    if (typeof parsed !== "object" || parsed === null) return fallback;
    const obj = parsed as Record<string, unknown>;
    const operator = obj.operator === "or" ? "or" : "and";
    const conds = Array.isArray(obj.conditions) ? obj.conditions : [];
    const conditions: UICondition[] = conds
      .filter((c): c is Record<string, unknown> => typeof c === "object" && c !== null)
      .map((c) => {
        const field = String(c.field ?? "kind") as FieldKey;
        const rawOp = String(c.op ?? "eq") as OpKey;
        // Normalize op to the field's allowed set so state is always valid;
        // without this, a persisted op that drifted from the current vocab
        // would stay in state and get re-serialized as-is.
        const def = FIELD_DEFS[field as FieldKey];
        const allowedOps = (def as { ops: readonly OpKey[] } | undefined)?.ops;
        const op: OpKey =
          allowedOps?.includes(rawOp) ? rawOp : (allowedOps?.[0] ?? "eq");
        const v = c.value;
        // `between` arrives as a 2-element array; everything else is
        // a scalar. Normalize for the form.
        if (Array.isArray(v)) {
          return {
            id: crypto.randomUUID(),
            field,
            op,
            value: v[0] != null ? String(v[0]) : "",
            value2: v[1] != null ? String(v[1]) : "",
          };
        }
        return {
          id: crypto.randomUUID(),
          field,
          op,
          value: v != null ? String(v) : "",
        };
      })
      .filter((c) => c.field in FIELD_DEFS);
    return {
      operator,
      conditions: conditions.length > 0 ? conditions : fallback.conditions,
    };
  } catch {
    return fallback;
  }
}

function serialize(rule: UIRule): string {
  return JSON.stringify(
    {
      operator: rule.operator,
      conditions: rule.conditions.map((c) => {
        const def = FIELD_DEFS[c.field];
        let value: unknown;
        if (c.op === "between") {
          value = [coerce(c.value, def.valueKind), coerce(c.value2 ?? "", def.valueKind)];
        } else if (c.op === "in") {
          value = c.value
            .split(",")
            .map((s) => s.trim())
            .filter((s) => s.length > 0)
            .map((s) => coerce(s, def.valueKind));
        } else {
          value = coerce(c.value, def.valueKind);
        }
        return { field: c.field, op: c.op, value };
      }),
    },
    null,
    2,
  );
}

function coerce(raw: string, kind: "int" | "real" | "text" | "enum"): unknown {
  if (kind === "int") {
    const n = Number.parseInt(raw, 10);
    return Number.isFinite(n) ? n : 0;
  }
  if (kind === "real") {
    const n = Number.parseFloat(raw);
    return Number.isFinite(n) ? n : 0;
  }
  return raw;
}

