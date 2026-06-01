import Link from "next/link";

/// "You / Server" switch that sits at the top of the settings sidebar. The
/// two contexts map to URL space — personal settings under /settings/* and
/// the admin console under /settings/admin/* — so each side is a real link
/// (prefetchable, shareable). "Server" only renders interactive for owners;
/// for everyone else it's a dimmed, inert label.
export function ContextSwitcher({
  context,
  canAccessServer,
  youHref = "/settings/account",
  serverHref = "/settings/admin",
}: {
  context: "you" | "server";
  canAccessServer: boolean;
  youHref?: string;
  serverHref?: string;
}) {
  const base =
    "flex-1 rounded px-3 py-1.5 text-center text-[12.5px] font-semibold transition-colors";
  return (
    <div className="flex gap-1 rounded-md border border-white/10 bg-white/4 p-1">
      <Link
        href={youHref}
        aria-current={context === "you" ? "page" : undefined}
        className={`${base} ${
          context === "you" ? "bg-white/10 text-white" : "text-white/60 hover:text-white"
        }`}
      >
        You
      </Link>
      {canAccessServer ? (
        <Link
          href={serverHref}
          aria-current={context === "server" ? "page" : undefined}
          className={`${base} ${
            context === "server"
              ? "bg-accent/15 text-accent ring-1 ring-inset ring-accent/30"
              : "text-white/60 hover:text-white"
          }`}
        >
          Server
        </Link>
      ) : (
        <span className={`${base} cursor-not-allowed text-white/25`}>Server</span>
      )}
    </div>
  );
}
