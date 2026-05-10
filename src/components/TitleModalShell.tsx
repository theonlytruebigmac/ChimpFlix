"use client";

import { useEffect } from "react";
import { closeModal } from "@/lib/modal";

export function TitleModalShell({ children }: { children: React.ReactNode }) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeModal();
    };
    window.addEventListener("keydown", handler);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", handler);
      document.body.style.overflow = previousOverflow;
    };
  }, []);

  return (
    <div
      onClick={closeModal}
      className="zf-modal-backdrop fixed inset-0 z-60 flex items-start justify-center overflow-y-auto bg-black/85 py-10"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="zf-modal-in relative w-full max-w-5xl overflow-hidden rounded-md bg-(--color-surface) shadow-2xl"
      >
        <button
          onClick={closeModal}
          aria-label="Close"
          className="absolute right-4 top-4 z-20 flex h-9 w-9 items-center justify-center rounded-full bg-black/70 text-white transition-colors hover:bg-black"
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
