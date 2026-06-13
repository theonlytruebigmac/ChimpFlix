"use client";

import { useState } from "react";
import Link from "next/link";
import type { User } from "@/lib/chimpflix-api";
import { SettingsProfileClient } from "./SettingsProfileClient";
import { SettingsEmailChangeClient } from "./SettingsEmailChangeClient";
import { SettingsPasswordClient } from "./SettingsPasswordClient";
import { SettingsTwoFactorClient } from "./SettingsTwoFactorClient";
import { SettingsLinkedAccountsClient } from "./SettingsLinkedAccountsClient";

type Tab = "profile" | "security" | "sessions" | "connections";

interface MembershipFacts {
  username: string;
  roleLabel: string;
  isOwner: boolean;
  joined: string;
  previousSignIn: string | null;
}

/// Account surface, rendered in the console design language to match the
/// redesign mockup (docs/redesign/account.html). In-page tabs switch
/// between Profile / Security / Sessions / Connections; each panel
/// composes the existing per-feature client components, which own all the
/// state, validation, and network calls. This wrapper only does the
/// tabbed layout and the read-only membership/sessions summaries.
export function SettingsAccountClient({
  user,
  membership,
}: {
  user: User;
  membership: MembershipFacts;
}) {
  const [tab, setTab] = useState<Tab>("profile");

  return (
    <div>
      {/* ── in-page tabs ──────────────────────────────────────────── */}
      <div className="cf-tabs" role="tablist" aria-label="Account sections">
        <TabButton id="profile" active={tab} onSelect={setTab}>
          Profile
        </TabButton>
        <TabButton id="security" active={tab} onSelect={setTab}>
          Security
        </TabButton>
        <TabButton id="sessions" active={tab} onSelect={setTab}>
          Sessions
        </TabButton>
        <TabButton id="connections" active={tab} onSelect={setTab}>
          Connections
        </TabButton>
      </div>

      {/* ── PROFILE ───────────────────────────────────────────────── */}
      {tab === "profile" && (
        <section role="tabpanel" id="tab-panel-profile" aria-labelledby="tab-btn-profile">
          <SettingsProfileClient initial={user} isOwner={membership.isOwner} />

          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Email</div>
                <div className="cf-sub">
                  Used for sign-in recovery and notification mirroring.
                </div>
              </div>
            </div>
            <div className="cf-card-body cf-pad">
              <SettingsEmailChangeClient initial={user} />
            </div>
          </div>

          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Membership</div>
                <div className="cf-sub">Read-only account facts.</div>
              </div>
            </div>
            <div className="cf-card-body">
              <div className="cf-grid cf-c2" style={{ padding: "14px 0" }}>
                <div>
                  <div className="cf-field-label">Username</div>
                  <div className="cf-mono">@{membership.username}</div>
                </div>
                <div>
                  <div className="cf-field-label">Role</div>
                  {membership.isOwner ? (
                    <span className="cf-pill cf-ok">
                      <span className="cf-dot" />
                      {membership.roleLabel}
                    </span>
                  ) : (
                    <span className="cf-pill">{membership.roleLabel}</span>
                  )}
                </div>
                <div>
                  <div className="cf-field-label">Joined</div>
                  <div>{membership.joined}</div>
                </div>
                {membership.previousSignIn && (
                  <div>
                    <div className="cf-field-label">Previous sign-in</div>
                    <div>{membership.previousSignIn}</div>
                  </div>
                )}
              </div>
            </div>
          </div>
        </section>
      )}

      {/* ── SECURITY ──────────────────────────────────────────────── */}
      {tab === "security" && (
        <section role="tabpanel" id="tab-panel-security" aria-labelledby="tab-btn-security">
          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Password</div>
                <div className="cf-sub">
                  Changing your password signs out every other device. You stay
                  signed in here.
                </div>
              </div>
            </div>
            <div className="cf-card-body cf-pad">
              <SettingsPasswordClient />
            </div>
          </div>

          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Two-factor authentication</div>
                <div className="cf-sub">
                  An authenticator app code in addition to your password.
                </div>
              </div>
            </div>
            <div className="cf-card-body cf-pad">
              <SettingsTwoFactorClient />
            </div>
          </div>
        </section>
      )}

      {/* ── SESSIONS ──────────────────────────────────────────────── */}
      {tab === "sessions" && (
        <section role="tabpanel" id="tab-panel-sessions" aria-labelledby="tab-btn-sessions">
          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Active sessions</div>
                <div className="cf-sub">
                  Sessions stay valid for 30 days. Manage every device you&rsquo;re
                  signed in on — including this one — and revoke anything you
                  don&rsquo;t recognise.
                </div>
              </div>
            </div>
            <div className="cf-card-body cf-pad">
              <Link className="cf-btn cf-primary" href="/settings/devices">
                <svg
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <rect x="2" y="4" width="20" height="13" rx="2" />
                  <path d="M8 20h8M12 17v3" />
                </svg>
                Manage devices &amp; sessions
              </Link>
            </div>
          </div>
        </section>
      )}

      {/* ── CONNECTIONS ───────────────────────────────────────────── */}
      {tab === "connections" && (
        <section role="tabpanel" id="tab-panel-connections" aria-labelledby="tab-btn-connections">
          <div className="cf-card">
            <div className="cf-card-head">
              <div>
                <div className="cf-ttl">Linked sign-in</div>
                <div className="cf-sub">
                  Sign in to ChimpFlix with a connected identity provider.
                </div>
              </div>
            </div>
            <div className="cf-card-body cf-pad">
              <SettingsLinkedAccountsClient />
            </div>
          </div>
          <div className="cf-banner cf-info" style={{ marginBottom: 0 }}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <circle cx="12" cy="12" r="9" />
              <path d="M12 8v.5M12 11v5" />
            </svg>
            <div>
              Unlinking keeps your account — you can sign back in with your
              password. If you never set one, use <b>Forgot password</b> first.
            </div>
          </div>
        </section>
      )}
    </div>
  );
}

function TabButton({
  id,
  active,
  onSelect,
  children,
}: {
  id: Tab;
  active: Tab;
  onSelect: (t: Tab) => void;
  children: React.ReactNode;
}) {
  const on = active === id;
  return (
    <button
      type="button"
      role="tab"
      id={`tab-btn-${id}`}
      aria-selected={on}
      aria-controls={`tab-panel-${id}`}
      className={"cf-tab" + (on ? " cf-on" : "")}
      onClick={() => onSelect(id)}
    >
      {children}
    </button>
  );
}
