"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function PersonError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Person page couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
