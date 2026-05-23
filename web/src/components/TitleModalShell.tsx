"use client";

import { useEffect, useRef } from "react";
import { closeModal } from "@/lib/modal";
import { useFocusTrap } from "@/lib/use-focus-trap";

export function TitleModalShell({ children }: { children: React.ReactNode }) {
  const cardRef = useRef<HTMLDivElement>(null);

  // Focus / Escape / restore handled by the shared hook.
  useFocusTrap(cardRef, { onClose: closeModal });

  // Body-scroll lock lives here because it's modal-specific (the
  // focus-trap hook is general-purpose and may be used by inline
  // popovers that shouldn't suppress page scroll).
  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, []);

  return (
    <div
      onClick={closeModal}
      role="dialog"
      aria-modal="true"
      aria-label="Title details"
      // Edge-to-edge on phones (no vertical padding eating screen
      // real estate); inset-with-padding on tablet+ for the classic
      // floating-card look. `safe-area-inset-bottom` extra padding so
      // the scrollable content can reach above the home-indicator on
      // iPhones without being clipped by the rounded-edge gesture bar.
      style={{ paddingBottom: "env(safe-area-inset-bottom, 0px)" }}
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
          // `safe-area-inset-top` margin so the button clears the
          // notch / status bar on iPhone X+ — without this the
          // close icon hides behind the dynamic island on landscape
          // playback. Falls back to 0 in browsers without env() so
          // the existing top-3 spacing still applies.
          style={{
            marginTop: "env(safe-area-inset-top, 0px)",
            marginRight: "env(safe-area-inset-right, 0px)",
          }}
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
