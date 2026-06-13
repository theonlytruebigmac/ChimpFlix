"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";

export interface CommandItem {
  href: string;
  label: string;
  /// Section the item belongs to ("You" / "Server"), shown right-aligned.
  group: string;
}

/// ⌘K command palette for the settings surface — type to jump to any page.
/// Controlled by the parent (SettingsShell owns the open state and the
/// global ⌘K listener); this owns the query, filtering, and navigation.
/// Phase 7 will extend it with arrow-key navigation and deep setting/action
/// targets; for now Enter selects the top match.
export function CommandPalette({
  open,
  onClose,
  items,
}: {
  open: boolean;
  onClose: () => void;
  items: CommandItem[];
}) {
  const router = useRouter();
  const [query, setQuery] = useState("");
  const [wasOpen, setWasOpen] = useState(open);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset the query each time the palette opens — adjusting state during
  // render is the React-endorsed alternative to a setState-in-effect.
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setQuery("");
  }

  useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => inputRef.current?.focus(), 20);
    return () => clearTimeout(t);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const q = query.toLowerCase().trim();
  const filtered = items.filter(
    (i) => !q || i.label.toLowerCase().includes(q) || i.group.toLowerCase().includes(q),
  );
  const go = (href: string) => {
    onClose();
    router.push(href);
  };

  return (
    <div
      className="fixed inset-0 z-60 flex items-start justify-center bg-black/60 pt-[14vh] backdrop-blur-sm"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Search settings"
    >
      <div
        className="w-full max-w-xl overflow-hidden rounded-xl border border-white/15 bg-(--color-surface-elevated) shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && filtered.length > 0) go(filtered[0].href);
          }}
          placeholder="Jump to a settings page…"
          className="w-full border-b border-white/10 bg-transparent px-4 py-3.5 text-base outline-none placeholder:text-white/30"
        />
        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filtered.length === 0 ? (
            <div className="px-3 py-3 text-sm text-white/40">No matches</div>
          ) : (
            filtered.map((i) => (
              <button
                key={`${i.group}:${i.href}:${i.label}`}
                type="button"
                onClick={() => go(i.href)}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-[13.5px] text-white/70 transition-colors hover:bg-white/8 hover:text-white"
              >
                <span>{i.label}</span>
                <span className="ml-auto text-[11.5px] text-white/30">{i.group}</span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
