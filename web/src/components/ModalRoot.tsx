"use client";

import { useEffect, useState } from "react";
import { TitleModalClient } from "./TitleModalClient";
import {
  MODAL_CLOSE_EVENT,
  MODAL_OPEN_EVENT,
  type OpenModalDetail,
} from "@/lib/modal";

const TITLE_PARAM = "title";
const EPISODE_PARAM = "ep";

function readFromLocation(): { titleId: string | null; episodeKey: string | null } {
  if (typeof window === "undefined") return { titleId: null, episodeKey: null };
  const sp = new URLSearchParams(window.location.search);
  return {
    titleId: sp.get(TITLE_PARAM),
    episodeKey: sp.get(EPISODE_PARAM),
  };
}

export function ModalRoot() {
  const [titleId, setTitleId] = useState<string | null>(null);
  // Episode hint that came along with the open. The modal uses this to
  // land on the right season and scroll the resume row into view when
  // the open is sourced from a Continue Watching tile.
  const [episodeKey, setEpisodeKey] = useState<string | null>(null);

  useEffect(() => {
    // Read the initial state from window.location on first client mount
    // — readFromLocation can't run during SSR, so this setState is the
    // synchronization point between URL-as-source-of-truth and React
    // state. All subsequent setState calls are event-driven (modal
    // open/close, popstate), which the lint rule is fine with.
    /* eslint-disable react-hooks/set-state-in-effect */
    const initial = readFromLocation();
    setTitleId(initial.titleId);
    setEpisodeKey(initial.episodeKey);
    /* eslint-enable react-hooks/set-state-in-effect */

    function onOpen(e: Event) {
      const detail = (e as CustomEvent<OpenModalDetail | string>).detail;
      // Backwards-compat: prior callers dispatched a bare string. New
      // callers pass `{ ratingKey, episodeKey }`.
      const ratingKey =
        typeof detail === "string" ? detail : detail?.ratingKey ?? null;
      const epKey =
        typeof detail === "string" ? null : detail?.episodeKey ?? null;
      if (!ratingKey) return;
      setTitleId(ratingKey);
      setEpisodeKey(epKey);
      const url = new URL(window.location.href);
      url.searchParams.set(TITLE_PARAM, ratingKey);
      if (epKey) url.searchParams.set(EPISODE_PARAM, epKey);
      else url.searchParams.delete(EPISODE_PARAM);
      window.history.pushState({}, "", url.toString());
    }

    function onClose() {
      setTitleId(null);
      setEpisodeKey(null);
      const url = new URL(window.location.href);
      url.searchParams.delete(TITLE_PARAM);
      url.searchParams.delete(EPISODE_PARAM);
      window.history.replaceState({}, "", url.toString());
    }

    function onPopState() {
      const next = readFromLocation();
      setTitleId(next.titleId);
      setEpisodeKey(next.episodeKey);
    }

    window.addEventListener(MODAL_OPEN_EVENT, onOpen);
    window.addEventListener(MODAL_CLOSE_EVENT, onClose);
    window.addEventListener("popstate", onPopState);
    return () => {
      window.removeEventListener(MODAL_OPEN_EVENT, onOpen);
      window.removeEventListener(MODAL_CLOSE_EVENT, onClose);
      window.removeEventListener("popstate", onPopState);
    };
  }, []);

  if (!titleId) return null;
  return (
    <TitleModalClient
      key={titleId}
      ratingKey={titleId}
      initialEpisodeKey={episodeKey ?? undefined}
    />
  );
}
