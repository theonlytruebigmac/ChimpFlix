"use client";

import { useEffect, useState } from "react";
import { TitleModalClient } from "./TitleModalClient";
import { MODAL_CLOSE_EVENT, MODAL_OPEN_EVENT } from "@/lib/modal";

const TITLE_PARAM = "title";

function readTitleFromLocation(): string | null {
  if (typeof window === "undefined") return null;
  return new URLSearchParams(window.location.search).get(TITLE_PARAM);
}

export function ModalRoot() {
  const [titleId, setTitleId] = useState<string | null>(null);

  useEffect(() => {
    setTitleId(readTitleFromLocation());

    function onOpen(e: Event) {
      const id = (e as CustomEvent<string>).detail;
      if (!id) return;
      setTitleId(id);
      const url = new URL(window.location.href);
      url.searchParams.set(TITLE_PARAM, id);
      window.history.pushState({}, "", url.toString());
    }

    function onClose() {
      setTitleId(null);
      const url = new URL(window.location.href);
      url.searchParams.delete(TITLE_PARAM);
      window.history.replaceState({}, "", url.toString());
    }

    function onPopState() {
      setTitleId(readTitleFromLocation());
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
  return <TitleModalClient key={titleId} ratingKey={titleId} />;
}
