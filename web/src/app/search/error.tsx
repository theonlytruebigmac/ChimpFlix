"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function SearchError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Search couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
