export function Brand({ size = "md" }: { size?: "sm" | "md" | "lg" }) {
  const cls =
    size === "lg"
      ? "text-4xl sm:text-5xl"
      : size === "sm"
        ? "text-xl sm:text-2xl"
        : "text-2xl sm:text-[1.65rem]";
  return (
    <span
      className={`select-none font-black tracking-tight text-(--color-accent) ${cls}`}
    >
      CHIMPFLIX
    </span>
  );
}
