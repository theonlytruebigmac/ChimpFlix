"use client";

import { useEffect } from "react";

/// Toggles the `cf-nav-scrolled` class on <body> based on scrollY.
/// TopNav's two background layers (gradient + solid) cross-fade off
/// that class via Tailwind's [body.cf-nav-scrolled_&] selector.
///
/// Mounted as a sibling of the nav rather than inline because we want
/// the parent <header> to remain server-rendered (no flash). The
/// effect itself runs on the client and is essentially free — one
/// passive scroll listener, no React renders.
const THRESHOLD_PX = 8;
const CLASS = "cf-nav-scrolled";

export function TopNavScrollEffect() {
  useEffect(() => {
    function sync() {
      const scrolled = window.scrollY > THRESHOLD_PX;
      document.body.classList.toggle(CLASS, scrolled);
    }
    sync();
    window.addEventListener("scroll", sync, { passive: true });
    return () => {
      window.removeEventListener("scroll", sync);
      document.body.classList.remove(CLASS);
    };
  }, []);
  return null;
}
