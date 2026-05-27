# V2 Auth — Identity, Sessions, Roles

> Status: **RFC skeleton.** Largely a port from V1.

## Scope

User authentication, session management, role hierarchy, OAuth
provider integration, invite-only signup.

## Carry forward from V1

V1 has a strong auth foundation laid in the recent Phase 80
(`user_auth_providers`). V2 ports it:

- **Three-tier role hierarchy.** Owner > Admin > User. `AdminAuth`
  extractor + `can_act_on` hierarchy guard + last-owner safety check.
- **Owner-only routes** for credentials, library mounts, destructive
  ops.
- **Username/password** with strong-password requirements.
- **2FA (TOTP)** with recovery codes.
- **Plex OAuth** (invite-only signup, password-less Plex-only users).
- **Provider-agnostic `user_auth_providers`** table for future Google
  OAuth and similar.
- **Cookie sessions, HMAC-signed.** Session secret from credential
  vault.
- **Email confirmation** flow.
- **Invite-only signup.**
- **MaybeAuthUser extractor** for routes that work both signed-in
  and anonymous.
- **Onboarding wizard** for first-run.

## What changes

- **Auth as a service, not entangled in handlers.** Auth checks live
  in middleware + extractors. Handlers see typed `AuthContext` and
  don't run their own permission checks.
- **Repository-layer access.** All auth-related writes go through
  typed repos (UserRepo, SessionRepo, InviteRepo).

## Open questions

- **Google OAuth.** V1 sketched the provider-agnostic shape; V2 may
  ship with Google OAuth from the start if operator demand exists.
  Decide based on user input.
- **Magic-link login.** Briefly considered for V1, not shipped.
  V2 worth revisiting: passwordless email links are pleasant UX,
  but add a dependency on email delivery quality. Defer unless
  requested.
- **Per-user library access controls.** V1 has hidden libraries.
  V2 considers per-user explicit allow-lists vs. the current model
  (hidden libraries hidden from non-owners).
- **API tokens.** For automation. V1 doesn't have these. V2 punts
  unless an operator surfaces a need.

## Cut list

- **OAuth2 server (ChimpFlix as provider).** Out of scope.
- **SAML / SSO.** Out of scope (single-operator deployment model).
- **WebAuthn / passkeys.** Worth considering eventually; not in V2
  initial cut.
