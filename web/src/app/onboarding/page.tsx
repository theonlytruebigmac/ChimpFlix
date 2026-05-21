import { redirect } from "next/navigation";
import { admin as adminApi } from "@/lib/chimpflix-api";
import { requireAdmin } from "@/lib/chimpflix-server";
import { OnboardingWizardClient } from "@/components/OnboardingWizardClient";

/// First-run wizard route. Owner/admin-only — regular viewers don't
/// need (and shouldn't see) the setup flow. If the wizard has
/// already been completed, redirect home so the URL isn't a
/// long-lived bookmark that re-opens it; admins can re-run the
/// flow from Admin → Server → General by flipping the
/// `setup_completed` setting back to false.
export default async function OnboardingPage() {
  await requireAdmin("/onboarding");
  const { settings } = await adminApi.settings.get();
  if (settings.setup_completed) {
    redirect("/");
  }
  return <OnboardingWizardClient initialSettings={settings} />;
}
