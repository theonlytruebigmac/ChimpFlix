"use client";

import { Component, type ReactNode } from "react";

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
/// responses, downstream render bugs). Failure → silent omission of
/// that rail; the rest of the page renders normally.
export class RailErrorBoundary extends Component<Props, State> {
  state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  componentDidCatch(error: unknown): void {
    console.error(`[RailErrorBoundary] ${this.props.label}:`, error);
  }

  render(): ReactNode {
    if (this.state.failed) return null;
    return this.props.children;
  }
}
