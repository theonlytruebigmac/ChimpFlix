"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function ResetPasswordError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Password reset couldn't load"
      fallbackHref="/login"
      fallbackLabel="Back to sign-in"
    />
  );
}
