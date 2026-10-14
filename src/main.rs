mod config;
mod discord;
mod geo;
mod hook;
mod notify;
mod slack;
mod sshd;
mod watch;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use config::{Config, Service};
use notify::Notifier;

#[derive(Parser)]
#[command(version, about = "Notifie sur Discord à chaque connexion SSH")]
struct Cli {
    /// Par défaut: ./config.toml, puis /etc/gabier/config.toml
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Affiche les notifications au lieu de les envoyer
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Surveille les logs (par défaut)
    Run {
        /// Pas de notification au démarrage du service
        #[arg(long)]
        quiet_start: bool,
    },
    /// Envoie un message de test
    Test,
    /// Vérifie la config et joint Discord, sans rien publier
    Check,
    /// Rejoue un fichier de log
    Replay { file: PathBuf },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let log = tracing_subscriber::fmt().with_env_filter(filter);
    // Sous systemd, journald horodate déjà chaque ligne.
    if std::env::var_os("JOURNAL_STREAM").is_some() {
        log.without_time().with_ansi(false).init();
    } else {
        log.init();
    }

    match cli.cmd.unwrap_or(Cmd::Run { quiet_start: false }) {
        Cmd::Run { quiet_start } => run(cli.config, cli.dry_run, quiet_start).await,
        Cmd::Replay { file } => replay(cli.config, cli.dry_run, &file).await,
        Cmd::Test => {
            let cfg = load(cli.config)?;
            let mut n = Notifier::new(cfg, cli.dry_run)?;
            n.announce(
                "✅ Gabier est branché",
                "Message de test. Si tu le vois, le webhook fonctionne.",
            )
            .await;
            if n.sent == 1 {
                println!("Envoyé. Regarde ton salon Discord.");
                Ok(())
            } else {
                bail!("rien n'est parti, voir les erreurs ci-dessus")
            }
        }
        Cmd::Check => check(cli.config).await,
    }
}

async fn run(path: Option<PathBuf>, dry_run: bool, quiet_start: bool) -> Result<()> {
    let cfg = load(path)?;
    let source = cfg.source.clone();
    let label = format!("{:?}", source.kind).to_lowercase();
    let mut n = Notifier::new(cfg, dry_run)?;
    info!(
        host = n.host(),
        source = %label,
        vers = %n.destinations().join("+"),
        "gabier démarre"
    );

    if !quiet_start {
        // Confirme que le service est bien revenu après un reboot.
        n.announce(
            "👀 Surveillance SSH démarrée",
            &format!("source `{label}`, version {}", env!("CARGO_PKG_VERSION")),
        )
        .await;
    }

    let (tx, mut rx) = mpsc::channel(4096);
    let src = tokio::spawn(watch::run(source, tx));

    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Un seul future pour tout la boucle: le recréer à chaque tour laisserait
    // passer un SIGTERM entre deux itérations.
    let stop = stop_signal();
    tokio::pin!(stop);

    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(l) => n.line(&l).await,
                None => { warn!("la source s'est arrêtée"); break }
            },
            _ = tick.tick() => n.tick().await,
            _ = &mut stop => { info!("arrêt demandé"); break }
        }
    }

    n.flush().await;
    src.abort();
    if let Ok(Err(e)) = src.await {
        return Err(e);
    }

    info!(
        lines = n.lines,
        sent = n.sent,
        held = n.held,
        "gabier s'arrête"
    );
    Ok(())
}

async fn replay(path: Option<PathBuf>, dry_run: bool, file: &Path) -> Result<()> {
    let cfg = load(path)?;
    let text = std::fs::read_to_string(file)?;
    let mut n = Notifier::new(cfg, dry_run)?;

    for line in text.lines() {
        n.line(line).await;
    }
    n.flush().await;

    println!(
        "\n{} lignes, {} évènements, {} notifications, {} échecs regroupés.",
        n.lines, n.events, n.sent, n.held
    );
    Ok(())
}

async fn check(path: Option<PathBuf>) -> Result<()> {
    let path = find_config(path)?;
    let cfg = Config::load(&path)?;

    println!("config    {}", path.display());
    println!("serveur   {}", cfg.host());
    match cfg.source.kind {
        config::SourceKind::Journald => {
            println!("source    journald ({})", cfg.source.units.join(", "))
        }
        config::SourceKind::File => println!("source    {}", cfg.source.path.display()),
        config::SourceKind::Stdin => println!("source    stdin"),
    }
    println!(
        "évènements accepted={} failed={} invalid_user={} closed={}",
        cfg.events.accepted, cfg.events.failed, cfg.events.invalid_user, cfg.events.closed
    );
    println!(
        "anti-spam {} échec(s) par IP, puis résumé toutes les {}s",
        cfg.throttle.burst, cfg.throttle.window_secs
    );
    println!("geoip     {}", cfg.geoip);

    // On teste toutes les destinations avant de rendre la main: `check` doit
    // dire tout ce qui ne va pas, pas seulement la première erreur.
    let mut bad = 0;
    for (svc, hook) in cfg.hooks() {
        let probed = match hook.url(svc) {
            Err(e) => Err(e),
            Ok(url) => {
                println!("{:9} {}", svc.name(), mask(&url));
                match svc {
                    Service::Discord => discord::probe(hook).await,
                    Service::Slack => slack::probe(hook).await,
                }
            }
        };
        match probed {
            Ok(()) => println!("          joignable"),
            Err(e) => {
                println!("          ✗ {e:#}");
                bad += 1;
            }
        }
    }

    if bad > 0 {
        bail!("{bad} destination(s) à corriger");
    }
    println!("\nTout est prêt.");
    Ok(())
}

fn load(path: Option<PathBuf>) -> Result<Config> {
    Config::load(&find_config(path)?)
}

fn find_config(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    for p in ["./config.toml", "/etc/gabier/config.toml"] {
        if Path::new(p).is_file() {
            return Ok(p.into());
        }
    }
    bail!("pas de config trouvée, passe --config")
}

/// On ne veut pas du jeton en clair dans la sortie de `check`.
fn mask(url: &str) -> String {
    match url.rsplit_once('/') {
        Some((head, token)) => format!("{head}/{}…", &token[..token.len().min(4)]),
        None => "?".into(),
    }
}

async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    #[test]
    fn masque_le_jeton() {
        let m = super::mask("https://discord.com/api/webhooks/123/abcdefghijkl");
        assert_eq!(m, "https://discord.com/api/webhooks/123/abcd…");
    }
}
