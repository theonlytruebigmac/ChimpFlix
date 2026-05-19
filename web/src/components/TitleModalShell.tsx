"use client";

import { useEffect, useRef } from "react";
import { closeModal } from "@/lib/modal";

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"]), input:not([disabled]), select:not([disabled]), textarea:not([disabled])';

export function TitleModalShell({ children }: { children: React.ReactNode }) {
  const cardRef = useRef<HTMLDivElement>(null);
  const previouslyFocusedRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    // Remember where focus was before the modal opened so we can
    // restore it on close (a11y baseline — keyboard users shouldn't
    // lose their place when a dialog dismisses).
    previouslyFocusedRef.current = document.activeElement as HTMLElement | null;
    const card = cardRef.current;
    if (card) {
      const first = card.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      first?.focus();
    }

    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        closeModal();
        return;
      }
      if (e.key !== "Tab") return;
      // Focus trap: cycle Tab and Shift+Tab inside the modal card so
      // keyboard users can't accidentally tab into the obscured page
      // content behind the backdrop. WCAG 2.1 dialog requirement.
      const c = cardRef.current;
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
    };
    window.addEventListener("keydown", handler);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", handler);
      document.body.style.overflow = previousOverflow;
      previouslyFocusedRef.current?.focus?.();
    };
  }, []);

  return (
    <div
      onClick={closeModal}
      role="dialog"
      aria-modal="true"
      // Edge-to-edge on phones (no vertical padding eating screen
      // real estate); inset-with-padding on tablet+ for the classic
      // floating-card look.
      className="zf-modal-backdrop fixed inset-0 z-60 flex items-start justify-center overflow-y-auto bg-black/85 sm:py-10"
    >
      <div
        ref={cardRef}
        onClick={(e) => e.stopPropagation()}
        // Full-width + no rounded corners on phones so the modal
        // doesn't waste pixels on a thin border. Rounded corners
        // return at sm+ where the modal floats above content.
        className="zf-modal-in relative w-full max-w-5xl overflow-hidden bg-(--color-surface) shadow-2xl sm:rounded-md"
      >
        <button
          onClick={closeModal}
          aria-label="Close"
          // 44×44 hit target on mobile (touch-friendly minimum);
          // 36×36 on desktop to match the smaller chrome density.
          className="absolute right-3 top-3 z-20 flex h-11 w-11 items-center justify-center rounded-full bg-black/70 text-white transition-colors hover:bg-black sm:right-4 sm:top-4 sm:h-9 sm:w-9"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.5"
            aria-hidden
          >
            <line x1="6" y1="6" x2="18" y2="18" />
            <line x1="18" y1="6" x2="6" y2="18" />
          </svg>
        </button>
        {children}
      </div>
    </div>
  );
}
