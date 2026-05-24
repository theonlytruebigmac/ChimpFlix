//! `chimpflix-server vault-rotate` — re-encrypt every vault-protected
//! row from an old key to a new key.
//!
//! Reads both keys from named environment variables (not flag values)
//! so the actual hex never lands in shell history or in `ps`.

use std::env;

use anyhow::{Context, Result, bail};
use chimpflix_common::{EncryptedBlob, Vault};
use chimpflix_library::{NewAuditEntry, queries};
use sqlx::{Row, SqlitePool};

use crate::cli;

pub async fn run(mut args: Vec<String>) -> Result<()> {
    let old_var = cli::pop_required(&mut args, "--old-key-env")?;
    let new_var = cli::pop_required(&mut args, "--new-key-env")?;
    let data_dir = cli::pop_data_dir(&mut args)?;
    if !args.is_empty() {
        bail!("unknown trailing args: {args:?}");
    }

    let old_hex = env::var(&old_var)
        .with_context(|| format!("env var {old_var} (old vault key) is not set"))?;
    let new_hex = env::var(&new_var)
        .with_context(|| format!("env var {new_var} (new vault key) is not set"))?;
    if old_hex == new_hex {
        bail!("old and new keys are identical — nothing to do");
    }

    let old_vault = Vault::from_hex_key(&old_hex).context("parse old vault key")?;
    let new_vault = Vault::from_hex_key(&new_hex).context("parse new vault key")?;

    cli::warn_server_running(&data_dir);

    let pool = chimpflix_library::open(&data_dir)
        .await
        .context("open library DB")?;

    let secrets = rotate_secrets(&pool, &old_vault, &new_vault).await?;
    let webhooks = rotate_webhooks(&pool, &old_vault, &new_vault).await?;
    let totps = rotate_user_totp(&pool, &old_vault, &new_vault).await?;

    let payload = serde_json::json!({
        "source": "cli",
        "secrets_rotated": secrets,
        "webhooks_rotated": webhooks,
        "user_totp_rotated": totps,
    });
    let _ = queries::append_audit(
        &pool,
        NewAuditEntry {
            actor_user_id: None,
            action: "cli.vault_rotate".into(),
            target_kind: Some("vault".into()),
            target_id: None,
            payload_json: Some(payload.to_string()),
            ip: None,
            user_agent: Some("chimpflix-server-cli".into()),
        },
    )
    .await;

    eprintln!(
        "Vault rotation complete:\n  \
         secrets:   {secrets} row(s) re-encrypted\n  \
         webhooks:  {webhooks} row(s) re-encrypted\n  \
         user_totp: {totps} row(s) re-encrypted\n\n\
         Restart the server with {new_var} set as CHIMPFLIX_SECRET_KEY.",
    );
    Ok(())
}

async fn rotate_secrets(pool: &SqlitePool, old: &Vault, new: &Vault) -> Result<usize> {
    let rows = sqlx::query("SELECT name, value_enc, nonce FROM secrets WHERE nonce IS NOT NULL")
        .fetch_all(pool)
        .await
        .context("scan secrets")?;

    let mut tx = pool.begin().await.context("begin secrets tx")?;
    let mut count = 0;
    for row in rows {
        let name: String = row.try_get("name")?;
        let value_enc: Vec<u8> = row.try_get("value_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("nonce").ok().flatten();
        let plaintext = old
            .decrypt(&EncryptedBlob {
                value: value_enc,
                nonce,
            })
            .with_context(|| format!("decrypt secrets.{name} with old key"))?;
        let blob = new
            .encrypt(&plaintext)
            .with_context(|| format!("encrypt secrets.{name} with new key"))?;
        sqlx::query("UPDATE secrets SET value_enc = ?, nonce = ? WHERE name = ?")
            .bind(blob.value)
            .bind(blob.nonce)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPDATE secrets.{name}"))?;
        count += 1;
    }
    tx.commit().await.context("commit secrets tx")?;
    Ok(count)
}

async fn rotate_webhooks(pool: &SqlitePool, old: &Vault, new: &Vault) -> Result<usize> {
    let rows = sqlx::query(
        "SELECT id, secret_enc, secret_nonce FROM webhooks
         WHERE secret_enc IS NOT NULL AND secret_nonce IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .context("scan webhooks")?;

    let mut tx = pool.begin().await.context("begin webhooks tx")?;
    let mut count = 0;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let value_enc: Vec<u8> = row.try_get("secret_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("secret_nonce").ok().flatten();
        let plaintext = old
            .decrypt(&EncryptedBlob {
                value: value_enc,
                nonce,
            })
            .with_context(|| format!("decrypt webhooks.id={id} with old key"))?;
        let blob = new
            .encrypt(&plaintext)
            .with_context(|| format!("encrypt webhooks.id={id} with new key"))?;
        sqlx::query("UPDATE webhooks SET secret_enc = ?, secret_nonce = ? WHERE id = ?")
            .bind(blob.value)
            .bind(blob.nonce)
            .bind(id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPDATE webhooks.id={id}"))?;
        count += 1;
    }
    tx.commit().await.context("commit webhooks tx")?;
    Ok(count)
}

async fn rotate_user_totp(pool: &SqlitePool, old: &Vault, new: &Vault) -> Result<usize> {
    let rows = sqlx::query(
        "SELECT user_id, secret_enc, secret_nonce FROM user_totp
         WHERE secret_nonce IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .context("scan user_totp")?;

    let mut tx = pool.begin().await.context("begin user_totp tx")?;
    let mut count = 0;
    for row in rows {
        let user_id: i64 = row.try_get("user_id")?;
        let value_enc: Vec<u8> = row.try_get("secret_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("secret_nonce").ok().flatten();
        let plaintext = old
            .decrypt(&EncryptedBlob {
                value: value_enc,
                nonce,
            })
            .with_context(|| format!("decrypt user_totp.user_id={user_id} with old key"))?;
        let blob = new
            .encrypt(&plaintext)
            .with_context(|| format!("encrypt user_totp.user_id={user_id} with new key"))?;
        sqlx::query("UPDATE user_totp SET secret_enc = ?, secret_nonce = ? WHERE user_id = ?")
            .bind(blob.value)
            .bind(blob.nonce)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("UPDATE user_totp.user_id={user_id}"))?;
        count += 1;
    }
    tx.commit().await.context("commit user_totp tx")?;
    Ok(count)
}
