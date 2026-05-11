"use client";

import { useEffect, useRef } from "react";
import { useParams, useRouter } from "next/navigation";
import { chimpflix } from "@/lib/api";

export default function WatchPage() {
  const params = useParams<{ fileId: string }>();
  const router = useRouter();
  const videoRef = useRef<HTMLVideoElement>(null);
  const fileId = Number(params.fileId);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") router.back();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [router]);

  if (!Number.isFinite(fileId)) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-black text-white">
        Invalid file id
      </div>
    );
  }

  return (
    <div className="fixed inset-0 bg-black">
      <button
        type="button"
        onClick={() => router.back()}
        className="absolute left-6 top-6 z-10 rounded-full bg-black/50 px-3 py-1.5 text-sm text-white/85 backdrop-blur-sm transition-colors hover:bg-black/70 hover:text-white"
      >
        ← Back
      </button>
      <video
        ref={videoRef}
        src={chimpflix.stream.directUrl(fileId)}
        controls
        autoPlay
        className="h-full w-full"
      />
    </div>
  );
}
