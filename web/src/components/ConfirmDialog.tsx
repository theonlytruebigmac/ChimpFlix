"use client";

import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

/// In-app replacement for `window.confirm`. Portal-rendered so it
/// escapes any transformed ancestor (TitleModalShell, etc.), focus-traps
/// on the primary action, closes on Escape, and supports destructive
/// styling for delete/merge/sign-out flows where the confirm button
/// should look dangerous.
///
/// Render this inline alongside the trigger, gated by a boolean state.
/// `onConfirm` runs first; if it throws, the caller is responsible for
/// surfacing the error — the dialog itself doesn't auto-close on error
/// (parent controls open state).
///
/// Example:
/// ```tsx
/// const [askDelete, setAskDelete] = useState(false);
/// <button onClick={() => setAskDelete(true)}>Delete</button>
/// {askDelete && (
///   <ConfirmDialog
///     title="Delete backup?"
///     body={`Delete ${name}? This cannot be undone.`}
///     confirmLabel="Delete"
///     destructive
///     onConfirm={async () => { await api.delete(id); setAskDelete(false); }}
///     onCancel={() => setAskDelete(false)}
///   />
/// )}
/// ```
export function ConfirmDialog({
  title,
  body,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  destructive = false,
  busy = false,
  confirmDisabled = false,
  onConfirm,
  onCancel,
}: {
  title: string;
  /// Either a string (rendered as one paragraph) or arbitrary JSX
  /// (lists, multi-paragraph callouts, etc.). Strings keep call sites
  /// terse; JSX lets us preserve the bulleted "what gets deleted"
  /// blocks from the existing `window.confirm` messages.
  body: React.ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  /// Style the confirm button as dangerous (red). Use for delete,
  /// merge, sign-out, revoke — anywhere the action is destructive.
  destructive?: boolean;
  /// When true, the confirm button shows a busy state and is disabled
  /// (Cancel still works). Hand back to the parent so it can wire the
  /// disabled state to a long-running promise.
  busy?: boolean;
  /// Disable the confirm button WITHOUT showing the busy spinner — for
  /// gating on a pre-action precondition (e.g. a typed-name match)
  /// rather than an in-flight request. Cancel stays enabled. `busy`
  /// still takes precedence for the spinner/label.
  confirmDisabled?: boolean;
  /// Called when the user clicks the confirm button. Parent is
  /// responsible for closing the dialog after the async action.
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
}) {
  const confirmRef = useRef<HTMLButtonElement | null>(null);

  // Focus the confirm button on mount so Enter activates it. We focus
  // confirm (not cancel) because the operator opened the dialog
  // deliberately; Escape is always the safe-out path.
  useEffect(() => {
    confirmRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel, busy]);

  if (typeof document === "undefined") return null;
  return createPortal(
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/70 p-4 zf-modal-backdrop"
      onClick={busy ? undefined : onCancel}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        className="zf-modal-in w-full max-w-md overflow-hidden rounded-lg border border-white/10 bg-(--color-surface) shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-6 pt-5 pb-2">
          <h2
            id="confirm-dialog-title"
            className="text-base font-semibold text-white"
          >
            {title}
          </h2>
        </div>
        <div className="px-6 pb-5 text-sm leading-relaxed text-white/75">
          {typeof body === "string" ? <p>{body}</p> : body}
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-white/10 bg-black/20 px-4 py-3">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="rounded-md border border-white/20 px-4 py-2 text-sm font-medium text-white/80 transition-colors hover:border-white/40 hover:text-white disabled:opacity-50"
          >
            {cancelLabel}
          </button>
          <button
            ref={confirmRef}
            type="button"
            onClick={() => void onConfirm()}
            disabled={busy || confirmDisabled}
            className={
              destructive
                ? "inline-flex items-center gap-2 rounded-md border border-red-500/40 bg-red-500/15 px-4 py-2 text-sm font-semibold text-red-200 transition-colors hover:border-red-500/65 hover:bg-red-500/25 disabled:opacity-50"
                : "inline-flex items-center gap-2 rounded-md bg-(--color-accent) px-4 py-2 text-sm font-semibold text-white transition-colors hover:opacity-90 disabled:opacity-50"
            }
          >
            {busy && (
              <svg
                className="h-3.5 w-3.5 animate-spin"
                viewBox="0 0 24 24"
                fill="none"
                aria-hidden
              >
                <circle
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeOpacity="0.25"
                  strokeWidth="3"
                />
                <path
                  d="M22 12a10 10 0 0 0-10-10"
                  stroke="currentColor"
                  strokeWidth="3"
                  strokeLinecap="round"
                />
              </svg>
            )}
            {busy ? "Working…" : confirmLabel}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
