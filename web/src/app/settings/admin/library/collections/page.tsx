import { AdminCollectionsClient } from "@/components/admin/AdminCollectionsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";
import { collections as collectionsApi } from "@/lib/chimpflix-api";

export default async function AdminCollectionsPage() {
  // Admin page wants to see + manage auto (TMDB) rows alongside the
  // user-curated ones. The home rail caller stays on the default
  // (manual + smart only) so auto franchises don't surface to users
  // until the operator has opted in to that experience.
  const { collections } = await collectionsApi.list({ include_auto: true });
  return (
    <div>
      <AdminPageHeader
        eyebrow="Library"
        title="Collections"
        description={
          <>
            Curated groupings that appear on Home and in the library
            browser. <strong className="text-white/80">Auto</strong>{" "}
            collections come from TMDB franchises (e.g. &ldquo;John Wick
            Collection&rdquo;) and are read-only;{" "}
            <strong className="text-white/80">manual</strong> collections
            are admin-authored.
          </>
        }
      />
      <AdminCollectionsClient initial={collections} />
    </div>
  );
}
