"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function GenreError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Genre page couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
