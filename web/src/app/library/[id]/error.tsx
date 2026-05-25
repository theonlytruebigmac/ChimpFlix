"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function LibraryError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Library couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
