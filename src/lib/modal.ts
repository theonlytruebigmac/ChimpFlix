// Tiny event-based API for the title modal. Avoids Next's router so opening
// and closing don't trigger a server roundtrip — both are pure React state
// updates that paint instantly. URL is kept in sync via the History API so
// links remain shareable and the back button still works.

const OPEN = "app:modal:open";
const CLOSE = "app:modal:close";

export function openModal(ratingKey: string): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent(OPEN, { detail: ratingKey }));
}

export function closeModal(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new Event(CLOSE));
}

export const MODAL_OPEN_EVENT = OPEN;
export const MODAL_CLOSE_EVENT = CLOSE;
