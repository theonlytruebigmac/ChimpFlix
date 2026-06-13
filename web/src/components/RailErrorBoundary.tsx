"use client";

import { Component, type ReactNode } from "react";
import { devError } from "@/lib/dev-log";

interface Props {
  /// Where the failure happened — surfaces in the console log so an
  /// operator skimming devtools can tell which rail blew up without
  /// having to look at the stack trace.
  label: string;
  children: ReactNode;
}

interface State {
  failed: boolean;
}

/// Catches render errors in a single home/browse rail so one broken
/// data source doesn't take down the whole page. Async server components
/// can't catch their own render errors; this client-side boundary picks
/// up anything that escapes Suspense (network failures, malformed API
/// responses, downstream render bugs).
///
/// On failure we render a subtle inline placeholder rather than
/// returning `null`. Silent omission was the old behavior; it left a
/// confusing gap on Home where a rail used to be, with no way for the
/// operator to know something had broken. The placeholder matches the
/// rail's vertical footprint (so the rest of the page doesn't reflow).
///
/// `label` is a developer diagnostic (e.g. `HomeHero`, `Lib:Anime`) and
/// is NEVER rendered to the user — it only goes to the console via
/// `devError` so an operator skimming devtools can tell which rail blew
/// up. It used to be painted as a visible `<h2>`, which leaked raw
/// component-ish names (`ContinueWatching`, `MovieGenre:Action`) onto the
/// home page. The card copy alone communicates the failure.
export class RailErrorBoundary extends Component<Props, State> {
  state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  componentDidCatch(error: unknown): void {
    devError(`[RailErrorBoundary] ${this.props.label}:`, error);
  }

  render(): ReactNode {
    if (this.state.failed) {
      return (
        <section className="px-4 sm:px-8 md:px-12 pb-1 pt-1">
          <div className="-mx-1 flex items-center gap-2 rounded-md border border-dashed border-white/15 bg-white/2 px-4 py-6 text-[12.5px] text-white/55">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="8" x2="12" y2="12" />
              <line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
            <span>
              This row couldn&apos;t load right now. The rest of the page is
              fine — check server logs for details.
            </span>
            {/* Resetting state re-renders children, letting Suspense re-execute the async fetch. */}
            <button
              onClick={() => this.setState({ failed: false })}
              className="ml-auto shrink-0 rounded px-2 py-0.5 text-[12.5px] text-white/55 underline-offset-2 hover:text-white/80 hover:underline"
            >
              Try again
            </button>
          </div>
        </section>
      );
    }
    return this.props.children;
  }
}
