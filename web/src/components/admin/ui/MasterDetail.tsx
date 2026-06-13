import type { ReactNode } from "react";

/// Two-pane master-detail layout: a scrollable list on the left and a
/// sticky detail pane on the right (collapses to a single column below
/// `lg`). Used by the new Libraries and Users pages. Compose with
/// `MasterList` + `MasterPane`; the pane typically wraps a `Drawer`.
export function MasterDetail({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_440px] lg:gap-6 ${className}`}
    >
      {children}
    </div>
  );
}

export function MasterList({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-2">{children}</div>;
}

export function MasterPane({ children }: { children: ReactNode }) {
  return <aside className="lg:sticky lg:top-28">{children}</aside>;
}
