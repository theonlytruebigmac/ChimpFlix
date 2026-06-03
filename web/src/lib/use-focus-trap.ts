"use client";

/// Standard dialog focus-trap hook. Pass a ref to the dialog
/// container; the hook will:
///
///   * Save the previously focused element on mount.
///   * Focus the first focusable element inside the container.
///   * Trap Tab + Shift-Tab cycling within the container so
///     keyboard users can't escape into the obscured page beneath.
///   * Optionally close on Escape.
///   * Restore focus to the previously focused element on unmount.
///
/// WCAG 2.1 dialog requirement. Extracted from TitleModalShell so
/// every dialog (EditMetadataDialog, FixMatchDialog, MarkerEditor,
/// HotkeysOverlay, etc.) can adopt the same behaviour without
/// duplicating the keyboard plumbing.

import { useEffect, useRef, type RefObject } from "react";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])';

interface Options {
  /// When true (default), Escape calls `onClose`. Set false for
  /// dialogs that own their own Escape handler (e.g. the player
  /// HotkeysOverlay, which the player's global keydown also toggles).
  closeOnEscape?: boolean;
  /// Called when Escape is pressed (only fires when `closeOnEscape`
  /// is true, which it is by default).
  onClose?: () => void;
  /// When true (default), the previously focused element is
  /// re-focused on unmount. Set false when the caller restores
  /// focus itself (e.g. animations that delay the unmount).
  restoreFocus?: boolean;
  /// When false (default true), skip the auto-focus-first-element
  /// step. Useful when the dialog has an obvious primary input the
  /// caller wants to focus manually.
  autoFocusFirst?: boolean;
}

export function useFocusTrap(
  containerRef: RefObject<HTMLElement | null>,
  options: Options = {},
): void {
  const {
    closeOnEscape = true,
    onClose,
    restoreFocus = true,
    autoFocusFirst = true,
  } = options;
  // Keep a ref to the latest onClose so the effect (which runs once)
  // always calls the current callback even when the parent re-renders
  // with a new function identity (e.g. inline arrow functions).
  const onCloseRef = useRef(onClose);
  useEffect(() => { onCloseRef.current = onClose; });
  useEffect(() => {
    const previouslyFocused = document.activeElement as HTMLElement | null;
    const container = containerRef.current;
    if (autoFocusFirst && container) {
      const first = container.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      first?.focus();
    }

    function onKey(e: KeyboardEvent) {
      if (closeOnEscape && e.key === "Escape") {
        onCloseRef.current?.();
        return;
      }
      if (e.key !== "Tab") return;
      const c = containerRef.current;
      if (!c) return;
      const focusable = Array.from(
        c.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      ).filter((el) => !el.hasAttribute("aria-hidden"));
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey) {
        if (active === first || !c.contains(active)) {
          e.preventDefault();
          last.focus();
        }
      } else {
        if (active === last || !c.contains(active)) {
          e.preventDefault();
          first.focus();
        }
      }
    }
    window.addEventListener("keydown", onKey);

    return () => {
      window.removeEventListener("keydown", onKey);
      if (restoreFocus && previouslyFocused && previouslyFocused.focus) {
        previouslyFocused.focus();
      }
    };
    // containerRef + options are referenced via closure; deliberate
    // empty-deps so a stable container that exists for the dialog's
    // lifetime doesn't churn the listener. Callers wanting to swap
    // the container should remount the consuming component.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
