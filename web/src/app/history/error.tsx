"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function HistoryError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="History couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
