"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function MyListError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="My List couldn't load"
      fallbackHref="/"
      fallbackLabel="Back to home"
    />
  );
}
