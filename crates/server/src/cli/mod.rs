//! Operator self-rescue subcommands.
//!
//! When the live server is dead in the water — sole owner forgot their
//! password, lost their TOTP device, rotated the vault key against an
//! old backup — these subcommands recover access without poking at
//! the SQLite file directly.
//!
//! Dispatch happens at the top of [`crate::main`]. If `argv[1]` matches
//! one of our subcommand names, the matching handler runs and the
//! process exits before any of the server's normal boot side effects
//! (tracing pipeline init, worker pool spin-up, axum bind) happen.
//!
//! All subcommands assume **the live server is stopped.** The DB is
//! opened with `chimpflix_library::open`, which acquires SQLite's
//! shared lock; a running server with active writes can collide. Each
//! handler prints a warning to that effect on entry.

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

mod owner_password;
mod owner_twofa;
mod vault_rotate;

/// Inspect `argv[1]`. If it names a subcommand, run it and return
/// `Some(result)`. Otherwise return `None` so [`crate::main`] continues
/// into the server boot path. Help and version flags also short-circuit
/// here.
pub async fn maybe_dispatch() -> Option<Result<()>> {
    let mut args = env::args().skip(1);
    let sub = args.next()?;
    match sub.as_str() {
        "owner-password-reset" => Some(owner_password::run(args.collect()).await),
        "owner-2fa-reset" => Some(owner_twofa::run(args.collect()).await),
        "vault-rotate" => Some(vault_rotate::run(args.collect()).await),
        "--help" | "-h" | "help" => {
            print_help();
            Some(Ok(()))
        }
        "--version" | "-V" | "version" => {
            println!("chimpflix-server {}", env!("CARGO_PKG_VERSION"));
            Some(Ok(()))
        }
        // Unknown leading arg: fall through to the server boot path,
        // which currently ignores argv. The server logs `BIND_ADDR`
        // etc. itself, so a typo'd subcommand won't silently start a
        // server — it'll be visible as "stray arg ignored" once
        // someone reads the next paragraph of logs.
        _ => None,
    }
}

fn print_help() {
    println!(
        "chimpflix-server {}

Run with no subcommand to start the HTTP server (default).

Operator self-rescue subcommands (require the live server to be stopped):

  owner-password-reset --email <addr> [--clear-2fa] [--data-dir <path>]
    Prompt for a new password and reset it for the named user. Pairs
    well with --clear-2fa when the operator has lost their TOTP device
    in the same incident. All active sessions for the user are
    invalidated.

  owner-2fa-reset --email <addr> [--data-dir <path>]
    Clear the user's TOTP secret + recovery codes. The next login for
    the account succeeds with the password alone; the user can
    re-enroll TOTP from Settings → Security. All active sessions for
    the user are invalidated.

  vault-rotate --old-key-env <NAME> --new-key-env <NAME> [--data-dir <path>]
    Decrypt every encrypted column with the old vault key and
    re-encrypt it with the new one. Reads both keys from the named
    environment variables to keep them out of shell history.

Common flags:
  --data-dir <path>   Override the data directory (default: $DATA_DIR or ./data).
  --help, -h          Show this help.
  --version, -V       Show the version string.
",
        env!("CARGO_PKG_VERSION"),
    );
}

/// Resolve `--data-dir` (or `$DATA_DIR`, or the `./data` default) and
/// strip the flag from `args` so the caller only sees its own flags.
pub(super) fn pop_data_dir(args: &mut Vec<String>) -> Result<PathBuf> {
    if let Some(i) = args.iter().position(|a| a == "--data-dir") {
        if i + 1 >= args.len() {
            bail!("--data-dir requires a value");
        }
        let value = args.remove(i + 1);
        args.remove(i);
        return Ok(PathBuf::from(value));
    }
    Ok(env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data")))
}

/// Pop the single value following `--<flag>` from `args`, returning it.
/// Errors if the flag is absent or has no value.
pub(super) fn pop_required(args: &mut Vec<String>, flag: &str) -> Result<String> {
    let i = args
        .iter()
        .position(|a| a == flag)
        .with_context(|| format!("{flag} is required"))?;
    if i + 1 >= args.len() {
        bail!("{flag} requires a value");
    }
    let value = args.remove(i + 1);
    args.remove(i);
    Ok(value)
}

/// Pop a value-less `--<flag>`. Returns `true` if it was present.
pub(super) fn pop_switch(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(i) = args.iter().position(|a| a == flag) {
        args.remove(i);
        true
    } else {
        false
    }
}

/// Print the "live server should be stopped" warning to stderr. Each
/// subcommand calls this on entry rather than trying to lock the DB
/// file: SQLite advisory locks let the open succeed against a running
/// server (it would just contend on writes), and a noisy human warning
/// is more useful than a cryptic SQLITE_BUSY.
pub(super) fn warn_server_running(data_dir: &std::path::Path) {
    eprintln!(
        "WARN: this subcommand writes to {}/chimpflix.db.\n      \
         Stop the live server (`docker compose stop chimpflix-server` or \
         systemctl stop chimpflix) before continuing,\n      \
         or accept the risk of SQLITE_BUSY conflicts.\n",
        data_dir.display(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_required_consumes_flag_and_value() {
        let mut args = vec![
            "--email".to_string(),
            "owner@example.com".to_string(),
            "--clear-2fa".to_string(),
        ];
        let v = pop_required(&mut args, "--email").unwrap();
        assert_eq!(v, "owner@example.com");
        assert_eq!(args, vec!["--clear-2fa".to_string()]);
    }

    #[test]
    fn pop_required_errors_when_absent() {
        let mut args = vec!["--other".to_string()];
        assert!(pop_required(&mut args, "--email").is_err());
    }

    #[test]
    fn pop_required_errors_on_missing_value() {
        let mut args = vec!["--email".to_string()];
        assert!(pop_required(&mut args, "--email").is_err());
    }

    #[test]
    fn pop_switch_returns_true_only_when_present() {
        let mut args = vec!["--clear-2fa".to_string(), "--email".to_string(), "x".to_string()];
        assert!(pop_switch(&mut args, "--clear-2fa"));
        assert_eq!(args, vec!["--email".to_string(), "x".to_string()]);
        assert!(!pop_switch(&mut args, "--missing"));
    }

    #[test]
    fn pop_data_dir_honours_flag() {
        let mut args = vec![
            "--data-dir".to_string(),
            "/tmp/cf-test".to_string(),
            "--email".to_string(),
        ];
        let dir = pop_data_dir(&mut args).unwrap();
        assert_eq!(dir, std::path::PathBuf::from("/tmp/cf-test"));
        assert_eq!(args, vec!["--email".to_string()]);
    }
}
