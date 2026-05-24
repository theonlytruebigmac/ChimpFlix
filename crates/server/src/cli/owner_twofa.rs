//! `chimpflix-server owner-2fa-reset` — clear a user's TOTP enrollment
//! + recovery codes from the command line.

use anyhow::{Context, Result, bail};
use chimpflix_library::{NewAuditEntry, queries};

use crate::cli;

pub async fn run(mut args: Vec<String>) -> Result<()> {
    let email = cli::pop_required(&mut args, "--email")?;
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
        "Clearing 2FA for user id={} username={} role={:?}",
        user.id, user.username, user.role,
    );

    let removed = queries::delete_user_totp(&pool, user.id)
        .await
        .context("delete TOTP for user")?;

    if removed {
        eprintln!("Cleared TOTP enrollment + recovery codes.");
    } else {
        eprintln!("No TOTP enrollment was present for this user.");
    }

    let revoked = queries::delete_sessions_for_user(&pool, user.id)
        .await
        .context("invalidate active sessions")?;
    eprintln!("Invalidated {revoked} active session(s).");

    // Audit-log so incident postmortems can reconstruct "who cleared
    // whose 2FA and when."
    let payload = serde_json::json!({
        "source": "cli",
        "target_email": email,
        "totp_removed": removed,
        "revoked_sessions": revoked,
    });
    let _ = queries::append_audit(
        &pool,
        NewAuditEntry {
            actor_user_id: None,
            action: "cli.owner_2fa_reset".into(),
            target_kind: Some("user".into()),
            target_id: Some(user.id.to_string()),
            payload_json: Some(payload.to_string()),
            ip: None,
            user_agent: Some("chimpflix-server-cli".into()),
        },
    )
    .await;

    eprintln!(
        "Done. The user's next login completes with password only.\n\
         Ask them to re-enroll TOTP from Settings -> Security."
    );
    Ok(())
}
