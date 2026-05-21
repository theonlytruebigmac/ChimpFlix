import { admin as adminApi } from "@/lib/chimpflix-api";
import { AdminCredentialsClient } from "@/components/admin/AdminCredentialsClient";
import { AdminPageHeader } from "@/components/admin/AdminPageHeader";

export default async function AdminCredentialsPage() {
  const initial = await adminApi.secrets.list();
  return (
    <div>
      <AdminPageHeader
        eyebrow="Server"
        title="Credentials"
        description={
          <>
            API keys for metadata agents and other integrations. Values
            are encrypted at rest when{" "}
            <code className="rounded bg-white/10 px-1.5 py-0.5 text-[11px]">
              CHIMPFLIX_SECRET_KEY
            </code>{" "}
            is set in the server&rsquo;s environment.
          </>
        }
      />
      <AdminCredentialsClient initial={initial} />
    </div>
  );
}
