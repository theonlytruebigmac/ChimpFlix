"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function CollectionError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Collection couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
