use std::io::SeekFrom;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::sync::mpsc::Sender;
use tracing::{info, warn};

use crate::config::{Source, SourceKind};

pub async fn run(cfg: Source, tx: Sender<String>) -> Result<()> {
    match cfg.kind {
        SourceKind::Journald => {
            info!(units = ?cfg.units, "je suis le journal systemd");
            journald(&cfg.units, tx).await
        }
        SourceKind::File => {
            info!(path = %cfg.path.display(), "je suis le fichier");
            tail(&cfg.path, tx).await
        }
        SourceKind::Stdin => {
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            while let Some(l) = lines.next_line().await? {
                if tx.send(l).await.is_err() {
                    break;
                }
            }
            Ok(())
        }
    }
}

async fn journald(units: &[String], tx: Sender<String>) -> Result<()> {
    let mut wait = Duration::from_secs(1);
    loop {
        let first_try = wait == Duration::from_secs(1);
        match journalctl(units, &tx).await {
            Ok(()) if tx.is_closed() => return Ok(()),
            Ok(()) => warn!("journalctl s'est arrêté, je relance dans {wait:?}"),
            // Le premier échec est le bon à remonter: journalctl absent,
            // ou pas le droit de lire le journal.
            Err(e) if first_try => return Err(e),
            Err(e) => warn!("journalctl: {e:#}, je relance dans {wait:?}"),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(60));
    }
}

async fn journalctl(units: &[String], tx: &Sender<String>) -> Result<()> {
    let mut cmd = tokio::process::Command::new("journalctl");
    cmd.args(["--follow", "--output=cat", "--lines=0", "--no-pager"]);
    for u in units {
        cmd.arg("--unit").arg(u);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().context(
        "journalctl introuvable — sur un système sans systemd, mets source.kind = \"file\"",
    )?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();

    let stderr = tokio::spawn(async move {
        let mut lines = BufReader::new(err).lines();
        let mut buf = String::new();
        while let Ok(Some(l)) = lines.next_line().await {
            buf.push_str(&l);
            buf.push('\n');
        }
        buf
    });

    let mut lines = BufReader::new(out).lines();
    let mut got = 0u64;
    while let Some(l) = lines.next_line().await? {
        if tx.send(l).await.is_err() {
            return Ok(());
        }
        got += 1;
    }

    let status = child.wait().await?;
    if !status.success() && got == 0 {
        let msg = stderr.await.unwrap_or_default();
        anyhow::bail!("journalctl a quitté avec {status}: {}", msg.trim());
    }
    Ok(())
}

/// tail -F: survit aux rotations de logrotate.
async fn tail(path: &Path, tx: Sender<String>) -> Result<()> {
    let mut file = None;
    let mut id = None;
    let mut pos = 0u64;
    let mut pending = Vec::new();
    let mut started = false;

    loop {
        if tx.is_closed() {
            return Ok(());
        }

        match tokio::fs::metadata(path).await {
            Ok(meta) => {
                let now_id = file_id(&meta);
                if file.is_some() && (meta.len() < pos || (now_id.is_some() && now_id != id)) {
                    info!("rotation détectée, je réouvre");
                    file = None;
                    pending.clear();
                }

                if file.is_none() {
                    let mut f = tokio::fs::File::open(path).await?;
                    // Au démarrage on part de la fin: pas question de notifier
                    // tout l'historique du fichier. Après une rotation, le
                    // nouveau fichier se lit depuis le début.
                    pos = if started { 0 } else { meta.len() };
                    f.seek(SeekFrom::Start(pos)).await?;
                    id = now_id;
                    started = true;
                    file = Some(f);
                }

                let f = file.as_mut().unwrap();
                let mut buf = [0u8; 16 * 1024];
                loop {
                    let n = f.read(&mut buf).await?;
                    if n == 0 {
                        break;
                    }
                    pos += n as u64;
                    pending.extend_from_slice(&buf[..n]);

                    // On ne découpe que sur des lignes complètes, sinon une
                    // lecture tombant au milieu d'un caractère UTF-8 le casse.
                    while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                        let raw: Vec<u8> = pending.drain(..=i).collect();
                        let line = String::from_utf8_lossy(&raw).trim().to_string();
                        if !line.is_empty() && tx.send(line).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
            Err(e) => warn!("{}: {e}", path.display()),
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(unix)]
fn file_id(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

// Faute d'inode, on se contente de la taille: suffisant pour développer.
#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> Option<(u64, u64)> {
    None
}
