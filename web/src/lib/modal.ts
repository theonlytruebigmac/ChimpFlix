// Tiny event-based API for the title modal. Avoids Next's router so opening
// and closing don't trigger a server roundtrip — both are pure React state
// updates that paint instantly. URL is kept in sync via the History API so
// links remain shareable and the back button still works.

const OPEN = "app:modal:open";
const CLOSE = "app:modal:close";

export type OpenModalDetail = { ratingKey: string; episodeKey?: string };

/// Opens the title modal for the given show / movie ratingKey. When
/// `episodeKey` is provided (a row like `e123`) and the modal target
/// is a show, the modal lands on the season containing that episode
/// and scrolls the row into view — i.e. opening from a Continue
/// Watching tile for S3E5 actually shows S3E5, not the show's S1.
export function openModal(ratingKey: string, episodeKey?: string): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent<OpenModalDetail>(OPEN, {
      detail: { ratingKey, episodeKey },
    }),
  );
}

export function closeModal(): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new Event(CLOSE));
}

export const MODAL_OPEN_EVENT = OPEN;
export const MODAL_CLOSE_EVENT = CLOSE;
