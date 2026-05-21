import { ModalRoot } from "@/components/ModalRoot";
import { MyListClient } from "@/components/MyListClient";
import { requireUser } from "@/lib/chimpflix-server";

export default async function MyListPage() {
  await requireUser("/my-list");

  return (
    <main className="relative min-h-screen bg-background">
      <MyListClient />
      <ModalRoot />
    </main>
  );
}
