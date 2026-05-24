//! `chimpflix-server owner-password-reset` — set a user's password
//! from the command line.

use anyhow::{Context, Result, bail};
use chimpflix_library::{NewAuditEntry, queries};

use crate::auth::password;
use crate::cli;

const MIN_PASSWORD_LEN: usize = 12;
const MAX_PASSWORD_LEN: usize = 1024;

pub async fn run(mut args: Vec<String>) -> Result<()> {
    let email = cli::pop_required(&mut args, "--email")?;
    let clear_2fa = cli::pop_switch(&mut args, "--clear-2fa");
    let data_dir = cli::pop_data_dir(&mut args)?;
    if !args.is_empty() {
        bail!("unknown trailing args: {args:?}");
    }

    cli::warn_server_running(&data_dir);

    let pool = chimpflix_library::open(&data_dir)
        .await
        .context("open library DB")?;

    let user = queries::find_user_by_email(&pool, &email)
        .await
        .context("look up user")?
        .with_context(|| format!("no user found with email {email}"))?;

    eprintln!(
        "Resetting password for user id={} username={} role={:?}",
        user.id, user.username, user.role,
    );

    let new_password = prompt_new_password()?;
    let hash = password::hash(&new_password).context("hash new password")?;

    let updated = queries::update_user_password(&pool, user.id, &hash)
        .await
        .context("update password")?;
    if !updated {
        bail!("update_user_password reported no rows affected");
    }

    if clear_2fa {
        let removed = queries::delete_user_totp(&pool, user.id)
            .await
            .context("clear TOTP for user")?;
        if removed {
            eprintln!("Cleared TOTP enrollment + recovery codes.");
        } else {
            eprintln!("No TOTP enrollment to clear.");
        }
    }

    let revoked = queries::delete_sessions_for_user(&pool, user.id)
        .await
        .context("invalidate active sessions")?;
    eprintln!("Invalidated {revoked} active session(s).");

    // Audit-log the action so incident postmortems can reconstruct
    // "who reset whose password and when." actor_user_id is None
    // because the CLI runs out-of-band of any HTTP session.
    let payload = serde_json::json!({
        "source": "cli",
        "target_email": email,
        "cleared_2fa": clear_2fa,
        "revoked_sessions": revoked,
    });
    let _ = queries::append_audit(
        &pool,
        NewAuditEntry {
            actor_user_id: None,
            action: "cli.owner_password_reset".into(),
            target_kind: Some("user".into()),
            target_id: Some(user.id.to_string()),
            payload_json: Some(payload.to_string()),
            ip: None,
            user_agent: Some("chimpflix-server-cli".into()),
        },
    )
    .await;

    eprintln!("Done. The user can now log in with the new password.");
    Ok(())
}

fn prompt_new_password() -> Result<String> {
    let first = rpassword::prompt_password("New password: ").context("read password")?;
    validate(&first)?;
    let confirm = rpassword::prompt_password("Confirm password: ").context("read password")?;
    if first != confirm {
        bail!("passwords did not match");
    }
    Ok(first)
}

fn validate(password: &str) -> Result<()> {
    if password.len() < MIN_PASSWORD_LEN {
        bail!("password must be at least {MIN_PASSWORD_LEN} characters");
    }
    if password.len() > MAX_PASSWORD_LEN {
        bail!("password must be at most {MAX_PASSWORD_LEN} characters");
    }
    // Mirror the runtime "all-same-char" guard from api::auth so the CLI
    // can't bypass the trivial-password floor that the HTTP handler
    // enforces. The full well-known-bad list lives in the handler; the
    // CLI is operator-only and we accept a slightly weaker check here.
    let first = password.as_bytes().first().copied();
    if let Some(c) = first {
        if password.as_bytes().iter().all(|&b| b == c) {
            bail!("password is a single repeated character — choose something less trivial");
        }
    }
    Ok(())
}
