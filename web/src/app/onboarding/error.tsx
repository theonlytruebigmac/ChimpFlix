"use client";

import { RouteErrorBoundary } from "@/components/RouteErrorBoundary";

export default function OnboardingError(props: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  return (
    <RouteErrorBoundary
      {...props}
      title="Onboarding hit a snag"
      fallbackHref="/login"
      fallbackLabel="Back to sign-in"
    />
  );
}
