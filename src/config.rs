use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::sshd::Kind;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    pub discord: Option<Hook>,
    pub slack: Option<Hook>,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub events: Events,
    #[serde(default)]
    pub throttle: Throttle,
    #[serde(default)]
    pub geoip: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Server {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Discord,
    Slack,
}

impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Service::Discord => "discord",
            Service::Slack => "slack",
        }
    }

    pub fn env(self) -> &'static str {
        match self {
            Service::Discord => "GABIER_DISCORD_WEBHOOK",
            Service::Slack => "GABIER_SLACK_WEBHOOK",
        }
    }

    fn looks_right(self, url: &str) -> bool {
        match self {
            Service::Discord => {
                url.contains("discord.com/api/webhooks/")
                    || url.contains("discordapp.com/api/webhooks/")
            }
            Service::Slack => url.contains("hooks.slack.com/services/"),
        }
    }
}

/// Même forme pour [discord] et [slack]: une URL de webhook, un nom
/// d'affichage, et qui pinger sur quoi.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    #[serde(default)]
    pub webhook_url: String,
    pub username: Option<String>,
    /// Discord: "<@123456789>". Slack: "<@U0123ABCD>" ou "<!channel>".
    pub ping: Option<String>,
    #[serde(default)]
    pub ping_on_accepted: bool,
    #[serde(default)]
    pub ping_on_failure: bool,
}

impl Hook {
    /// La variable d'environnement gagne sur le fichier.
    pub fn url(&self, svc: Service) -> Result<String> {
        let url = match std::env::var(svc.env()) {
            Ok(v) if !v.trim().is_empty() => v,
            _ if !self.webhook_url.trim().is_empty() => self.webhook_url.clone(),
            _ => bail!(
                "{}: pas de webhook, renseigne webhook_url ou exporte {}",
                svc.name(),
                svc.env()
            ),
        };
        let url = url.trim().to_string();
        if !svc.looks_right(&url) {
            bail!("{}: ça ne ressemble pas au bon webhook: {url}", svc.name());
        }
        Ok(url)
    }

    pub fn ping_for(&self, kind: Kind) -> Option<String> {
        let wanted = match kind {
            Kind::Accepted => self.ping_on_accepted,
            Kind::Failed | Kind::InvalidUser => self.ping_on_failure,
            Kind::Closed => false,
        };
        self.ping.clone().filter(|_| wanted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Journald,
    File,
    Stdin,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Source {
    pub kind: SourceKind,
    pub units: Vec<String>,
    pub path: PathBuf,
}

impl Default for Source {
    fn default() -> Self {
        Source {
            kind: SourceKind::Journald,
            // "ssh" sur Debian, "sshd" ailleurs. journalctl ignore
            // tranquillement une unité qui n'existe pas.
            units: vec!["ssh".into(), "sshd".into()],
            path: "/var/log/auth.log".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Events {
    pub accepted: bool,
    pub failed: bool,
    pub invalid_user: bool,
    pub closed: bool,
}

impl Default for Events {
    fn default() -> Self {
        Events {
            accepted: true,
            failed: true,
            invalid_user: true,
            closed: false,
        }
    }
}

impl Events {
    pub fn wants(&self, kind: Kind) -> bool {
        match kind {
            Kind::Accepted => self.accepted,
            Kind::Failed => self.failed,
            Kind::InvalidUser => self.invalid_user,
            Kind::Closed => self.closed,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Throttle {
    pub window_secs: u64,
    pub burst: u32,
}

impl Default for Throttle {
    fn default() -> Self {
        Throttle {
            window_secs: 300,
            burst: 1,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("lecture de {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&raw).with_context(|| format!("config invalide: {}", path.display()))?;

        if cfg.discord.is_none() && cfg.slack.is_none() {
            bail!("aucune destination: ajoute une section [discord] ou [slack]");
        }
        if cfg.throttle.window_secs == 0 {
            bail!("throttle.window_secs doit être > 0");
        }
        if cfg.source.kind == SourceKind::Journald && cfg.source.units.is_empty() {
            bail!("source.units est vide");
        }
        Ok(cfg)
    }

    /// Les destinations configurées, dans l'ordre d'affichage.
    pub fn hooks(&self) -> Vec<(Service, &Hook)> {
        [
            (Service::Discord, self.discord.as_ref()),
            (Service::Slack, self.slack.as_ref()),
        ]
        .into_iter()
        .filter_map(|(svc, h)| h.map(|h| (svc, h)))
        .collect()
    }

    pub fn host(&self) -> String {
        if let Some(n) = self.server.name.as_ref().filter(|n| !n.trim().is_empty()) {
            return n.trim().to_string();
        }
        hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "?".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(extra: &str) -> Config {
        let src =
            format!("[discord]\nwebhook_url = \"https://discord.com/api/webhooks/1/x\"\n{extra}");
        toml::from_str(&src).unwrap()
    }

    #[test]
    fn defauts() {
        let c = parse("");
        assert_eq!(c.source.kind, SourceKind::Journald);
        assert_eq!(c.source.units, ["ssh", "sshd"]);
        assert_eq!(c.throttle.window_secs, 300);
        assert!(c.events.accepted && !c.events.closed);
        assert!(!c.geoip);
        assert!(c.slack.is_none());
    }

    #[test]
    fn surcharges() {
        let c = parse(
            r#"
            [source]
            kind = "file"
            path = "/var/log/secure"

            [events]
            closed = true

            [throttle]
            burst = 3
            "#,
        );
        assert_eq!(c.source.kind, SourceKind::File);
        assert_eq!(c.source.path, PathBuf::from("/var/log/secure"));
        assert!(c.events.closed);
        assert_eq!(c.throttle.burst, 3);
        // un [events] partiel garde les autres défauts
        assert!(c.events.accepted);
        // et [source] partiel aussi
        assert_eq!(c.source.units, ["ssh", "sshd"]);
    }

    #[test]
    fn typo_rejetee() {
        let src = "[discord]\nwebhook_url = \"x\"\n\n[throtle]\nburst = 1\n";
        assert!(toml::from_str::<Config>(src).is_err());
    }

    #[test]
    fn ping_cible() {
        let mut h = parse("").discord.unwrap();
        h.ping = Some("<@42>".into());
        h.ping_on_accepted = true;
        assert_eq!(h.ping_for(Kind::Accepted).as_deref(), Some("<@42>"));
        assert_eq!(h.ping_for(Kind::Failed), None);
        assert_eq!(h.ping_for(Kind::Closed), None);
    }

    #[test]
    fn urls_verifiees_par_service() {
        let h = Hook {
            webhook_url: "https://hooks.slack.com/services/T0/B0/xxx".into(),
            username: None,
            ping: None,
            ping_on_accepted: false,
            ping_on_failure: false,
        };
        assert!(h.url(Service::Slack).is_ok());
        // la même URL n'est pas un webhook Discord
        assert!(h.url(Service::Discord).is_err());
    }

    #[test]
    fn les_deux_destinations() {
        let c: Config = toml::from_str(
            r#"
            [discord]
            webhook_url = "https://discord.com/api/webhooks/1/x"

            [slack]
            webhook_url = "https://hooks.slack.com/services/T0/B0/x"
            ping = "<!channel>"
            "#,
        )
        .unwrap();
        let hooks = c.hooks();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].0, Service::Discord);
        assert_eq!(hooks[1].0, Service::Slack);
    }
}
