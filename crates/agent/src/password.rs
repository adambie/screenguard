use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::users::scan_local_users;

async fn username_for_uid(uid: u32) -> Result<String> {
    scan_local_users(0)?
        .into_iter()
        .find(|user| user.local_uid == uid)
        .map(|user| user.username)
        .with_context(|| format!("No local login user found for uid={uid}"))
}

async fn password_is_locked(username: &str) -> Result<bool> {
    let output = Command::new("passwd")
        .args(["--status", username])
        .output()
        .await
        .with_context(|| format!("Failed to read password status for {username}"))?;
    if !output.status.success() {
        bail!(
            "passwd --status failed for {username}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1) == Some("L"))
}

/// Enable or disable password authentication for a local account.
pub async fn set_password_locked(uid: u32, locked: bool) -> Result<()> {
    let username = username_for_uid(uid).await?;
    if locked && password_is_locked(&username).await? {
        tracing::info!("Password login is already disabled for uid={uid}");
        return Ok(());
    }
    let action = if locked { "--lock" } else { "--unlock" };
    let output = Command::new("passwd")
        .args([action, &username])
        .output()
        .await
        .with_context(|| format!("Failed to run passwd {action} for {username}"))?;

    if !output.status.success() {
        bail!(
            "passwd {action} failed for {username}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    tracing::info!(
        "{} password login for uid={uid}",
        if locked { "Disabled" } else { "Enabled" },
    );
    Ok(())
}
