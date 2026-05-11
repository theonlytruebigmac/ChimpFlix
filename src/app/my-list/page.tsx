import { redirect } from "next/navigation";
import { readToken } from "@/lib/session";
import { ModalRoot } from "@/components/ModalRoot";
import { MyListClient } from "@/components/MyListClient";
import { TopNav } from "@/components/TopNav";

export default async function MyListPage() {
  const token = await readToken();
  if (!token) redirect("/login");

  return (
    <main className="relative min-h-screen bg-black">
      <TopNav />
      <MyListClient />
      <ModalRoot />
    </main>
  );
}
