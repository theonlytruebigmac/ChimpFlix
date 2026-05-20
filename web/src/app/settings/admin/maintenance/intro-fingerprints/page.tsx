import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminIntroFingerprintsClient } from "@/components/admin/AdminIntroFingerprintsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminIntroFingerprintsPage() {
  const initial = await adminApi.introFingerprints.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Maintenance"
        title="Intro fingerprints"
        description="Per-show audio signatures that drive the intro detector. Chapter-derived intros seed auto fingerprints; saving a manual intro marker on episode 1 of a show produces a higher-confidence manual fingerprint."
      />
      <AdminIntroFingerprintsClient initial={initial.fingerprints} />
    </div>
  );
}
