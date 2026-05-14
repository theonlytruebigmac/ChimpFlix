// Server-side helpers used by Server Components. Centralizes auth
// resolution against the Rust backend so individual pages don't repeat
// the try/catch/redirect dance.

import "server-only";
import { redirect } from "next/navigation";
import {
  ChimpFlixApiError,
  auth as authApi,
  type User,
} from "./chimpflix-api";

export async function requireUser(currentPath: string): Promise<User> {
  try {
    const { user } = await authApi.me();
    return user;
  } catch (e) {
    if (e instanceof ChimpFlixApiError && e.status === 401) {
      redirect(`/login?next=${encodeURIComponent(currentPath)}`);
    }
    throw e;
  }
}

/// Resolves the current user and verifies they hold the `owner` role.
/// Non-owners are bounced to the user-facing settings page so they see a
/// useful destination instead of a bare 403.
export async function requireOwner(currentPath: string): Promise<User> {
  const user = await requireUser(currentPath);
  if (user.role !== "owner") {
    redirect("/settings");
  }
  return user;
}
