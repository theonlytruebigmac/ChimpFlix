"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function NewPopularError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="New &amp; popular couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
