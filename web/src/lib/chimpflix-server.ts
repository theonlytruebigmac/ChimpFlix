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
/// useful destination instead of a bare 403. Use for the most sensitive
/// pages — credentials, library mounts, server URLs, owner-role
/// mutations.
export async function requireOwner(currentPath: string): Promise<User> {
  const user = await requireUser(currentPath);
  if (user.role !== "owner") {
    redirect("/settings");
  }
  return user;
}

/// Resolves the current user and verifies they hold either `owner` or
/// `admin` role. Use for routine admin-surface pages (Users, Access,
/// Groups, Library Settings, Tasks, etc.) — handlers further enforce
/// the role hierarchy when acting on a specific target so admins
/// never modify owner accounts.
export async function requireAdmin(currentPath: string): Promise<User> {
  const user = await requireUser(currentPath);
  if (user.role !== "owner" && user.role !== "admin") {
    redirect("/settings");
  }
  return user;
}
