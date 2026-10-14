use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use tracing::{debug, error, info};

use crate::config::{Config, Service};
use crate::discord::Discord;
use crate::geo::Geo;
use crate::hook::Msg;
use crate::slack::Slack;
use crate::sshd::{self, Event, Kind};

/// sshd écrit deux lignes pour une même tentative sur un compte inconnu
/// ("Invalid user x", puis "Failed password for invalid user x"). Même IP,
/// même port source, même seconde: c'est un doublon.
const DEDUP: Duration = Duration::from_secs(10);

enum Sink {
    Discord(Discord),
    Slack(Slack),
    Print,
}

impl Sink {
    async fn send(&self, msg: &Msg) -> Result<()> {
        match self {
            Sink::Discord(d) => d.send(msg).await,
            Sink::Slack(s) => s.send(msg).await,
            Sink::Print => {
                println!("{}", msg.text());
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Sink::Discord(_) => "discord",
            Sink::Slack(_) => "slack",
            Sink::Print => "stdout",
        }
    }
}

struct Burst {
    start: Instant,
    seen: u32,
    sent: u32,
    users: BTreeSet<String>,
}

pub struct Notifier {
    cfg: Config,
    host: String,
    sinks: Vec<Sink>,
    geo: Option<Geo>,
    recent: HashMap<(Kind, String, u16), Instant>,
    bursts: HashMap<IpAddr, Burst>,
    pub lines: u64,
    pub events: u64,
    pub sent: u64,
    pub held: u64,
}

impl Notifier {
    pub fn new(cfg: Config, dry_run: bool) -> Result<Self> {
        let mut sinks = Vec::new();
        if dry_run {
            sinks.push(Sink::Print);
        } else {
            for (svc, hook) in cfg.hooks() {
                sinks.push(match svc {
                    Service::Discord => Sink::Discord(Discord::new(hook)?),
                    Service::Slack => Sink::Slack(Slack::new(hook)?),
                });
            }
            if sinks.is_empty() {
                bail!("aucune destination: ajoute une section [discord] ou [slack]");
            }
        }

        Ok(Notifier {
            host: cfg.host(),
            geo: cfg.geoip.then(Geo::new),
            cfg,
            sinks,
            recent: HashMap::new(),
            bursts: HashMap::new(),
            lines: 0,
            events: 0,
            sent: 0,
            held: 0,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn destinations(&self) -> Vec<&'static str> {
        self.sinks.iter().map(Sink::name).collect()
    }

    pub async fn line(&mut self, line: &str) {
        self.lines += 1;
        let Some(ev) = sshd::parse(line) else { return };
        self.events += 1;

        if !self.cfg.events.wants(ev.kind) {
            return;
        }
        if self.is_dup(&ev) {
            debug!("doublon: {} {}", ev.user, ev.ip_str());
            return;
        }

        info!("{:?} user={} ip={}", ev.kind, ev.user, ev.ip_str());

        match (ev.kind.is_failure(), ev.ip) {
            (true, Some(ip)) => self.count_failure(ip, &ev).await,
            _ => self.send_event(&ev).await,
        }
    }

    fn is_dup(&mut self, ev: &Event) -> bool {
        let Some(port) = ev.port.filter(|_| ev.kind.is_failure()) else {
            return false;
        };
        let key = (ev.kind, ev.ip_str(), port);
        let now = Instant::now();
        match self.recent.insert(key, now) {
            Some(before) => now - before < DEDUP,
            None => false,
        }
    }

    /// Les échecs ne partent pas un par un: les `burst` premiers d'une IP
    /// passent, le reste est compté et résumé à la fermeture de la fenêtre.
    async fn count_failure(&mut self, ip: IpAddr, ev: &Event) {
        let window = Duration::from_secs(self.cfg.throttle.window_secs);
        let now = Instant::now();

        let b = self.bursts.entry(ip).or_insert_with(|| Burst {
            start: now,
            seen: 0,
            sent: 0,
            users: BTreeSet::new(),
        });
        if now - b.start >= window {
            *b = Burst {
                start: now,
                seen: 0,
                sent: 0,
                users: BTreeSet::new(),
            };
        }

        b.seen += 1;
        b.users.insert(ev.user.clone());
        let pass = b.sent < self.cfg.throttle.burst;
        if pass {
            b.sent += 1;
        }

        if pass {
            self.send_event(ev).await;
        } else {
            self.held += 1;
        }
    }

    /// À appeler régulièrement: publie les résumés des fenêtres échues.
    pub async fn tick(&mut self) {
        self.sweep(false).await
    }

    /// À l'arrêt, on vide ce qui reste en attente.
    pub async fn flush(&mut self) {
        self.sweep(true).await
    }

    async fn sweep(&mut self, all: bool) {
        let window = Duration::from_secs(self.cfg.throttle.window_secs);
        let now = Instant::now();
        self.recent.retain(|_, t| now - *t < DEDUP);

        let done: Vec<IpAddr> = self
            .bursts
            .iter()
            .filter(|(_, b)| all || now - b.start >= window)
            .map(|(ip, _)| *ip)
            .collect();

        for ip in done {
            let b = self.bursts.remove(&ip).unwrap();
            if b.seen <= b.sent {
                continue;
            }

            let users: Vec<&str> = b.users.iter().take(15).map(|s| s.as_str()).collect();
            let mut list = users.join(", ");
            if b.users.len() > 15 {
                list += &format!(" (+{})", b.users.len() - 15);
            }

            let mut fields = vec![
                ("IP".into(), format!("`{ip}`")),
                ("Tentatives".into(), b.seen.to_string()),
                ("Comptes visés".into(), list),
            ];
            if let Some(g) = self.locate(Some(ip)).await {
                fields.push(("Origine".into(), g));
            }

            self.send(Msg {
                title: "🚨 Rafale de tentatives SSH".into(),
                color: Kind::InvalidUser.color(),
                body: Some(format!("{} tentatives refusées depuis `{ip}`.", b.seen)),
                fields,
                footer: self.host.clone(),
                ping: Some(Kind::Failed),
            })
            .await;
        }
    }

    /// Message de service (test, démarrage): pas un évènement SSH.
    pub async fn announce(&mut self, title: &str, body: &str) {
        self.send(Msg {
            title: title.into(),
            color: 0x3498db,
            body: Some(body.into()),
            fields: vec![("Serveur".into(), format!("`{}`", self.host))],
            footer: self.host.clone(),
            ping: None,
        })
        .await;
    }

    async fn send_event(&mut self, ev: &Event) {
        let mut fields = vec![
            ("Utilisateur".into(), format!("`{}`", ev.user)),
            ("IP".into(), format!("`{}`", ev.ip_str())),
        ];
        if let Some(m) = &ev.method {
            fields.push(("Méthode".into(), format!("`{m}`")));
        }
        if let Some(p) = ev.port {
            fields.push(("Port".into(), p.to_string()));
        }
        if let Some(g) = self.locate(ev.ip).await {
            fields.push(("Origine".into(), g));
        }

        self.send(Msg {
            title: ev.kind.title().into(),
            color: ev.kind.color(),
            body: ev.key.as_ref().map(|k| format!("Clé `{k}`")),
            fields,
            footer: self.host.clone(),
            ping: Some(ev.kind),
        })
        .await;
    }

    /// Compte une notification dès qu'une destination l'a prise: si Slack
    /// tombe, la surveillance continue sur Discord.
    async fn send(&mut self, msg: Msg) {
        let mut ok = false;
        for sink in &self.sinks {
            match sink.send(&msg).await {
                Ok(()) => ok = true,
                Err(e) => error!("{}: {e:#}", sink.name()),
            }
        }
        if ok {
            self.sent += 1;
        }
    }

    async fn locate(&mut self, ip: Option<IpAddr>) -> Option<String> {
        self.geo.as_mut()?.lookup(ip?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notifier(extra: &str) -> Notifier {
        let src = format!(
            "[server]\nname = \"vps\"\n\n[discord]\nwebhook_url = \"https://discord.com/api/webhooks/1/x\"\n{extra}"
        );
        Notifier::new(toml::from_str(&src).unwrap(), true).unwrap()
    }

    #[tokio::test]
    async fn accepted_part_tout_de_suite() {
        let mut n = notifier("");
        n.line("Accepted publickey for root from 203.0.113.9 port 22 ssh2")
            .await;
        assert_eq!((n.events, n.sent, n.held), (1, 1, 0));
    }

    #[tokio::test]
    async fn lignes_hors_sujet() {
        let mut n = notifier("");
        n.line("Server listening on 0.0.0.0 port 22.").await;
        n.line("").await;
        assert_eq!((n.lines, n.events, n.sent), (2, 0, 0));
    }

    #[tokio::test]
    async fn rafale_resumee() {
        let mut n = notifier("[throttle]\nburst = 1");
        for port in 50000..50010 {
            n.line(&format!(
                "Failed password for invalid user admin from 192.0.2.66 port {port} ssh2"
            ))
            .await;
        }
        assert_eq!((n.sent, n.held), (1, 9));

        n.flush().await;
        assert_eq!(n.sent, 2, "un résumé en fin de fenêtre");
    }

    #[tokio::test]
    async fn burst_regle() {
        let mut n = notifier("[throttle]\nburst = 3");
        for port in 50000..50010 {
            n.line(&format!(
                "Failed password for root from 192.0.2.66 port {port} ssh2"
            ))
            .await;
        }
        assert_eq!((n.sent, n.held), (3, 7));
    }

    #[tokio::test]
    async fn compteurs_par_ip() {
        let mut n = notifier("");
        n.line("Failed password for root from 192.0.2.1 port 1 ssh2")
            .await;
        n.line("Failed password for root from 192.0.2.2 port 2 ssh2")
            .await;
        assert_eq!((n.sent, n.held), (2, 0));
    }

    #[tokio::test]
    async fn doublon_sshd() {
        let mut n = notifier("[throttle]\nburst = 5");
        n.line("Invalid user admin from 192.0.2.66 port 50000")
            .await;
        n.line("Failed password for invalid user admin from 192.0.2.66 port 50000 ssh2")
            .await;
        assert_eq!((n.events, n.sent), (2, 1));
    }

    #[tokio::test]
    async fn evenements_filtres() {
        let mut n = notifier("[events]\naccepted = false");
        n.line("Accepted password for root from 203.0.113.9 port 22 ssh2")
            .await;
        assert_eq!((n.events, n.sent), (1, 0));

        // closed est à false par défaut
        let mut n = notifier("");
        n.line("Disconnected from user root 203.0.113.9 port 22")
            .await;
        assert_eq!(n.sent, 0);
        let mut n = notifier("[events]\nclosed = true");
        n.line("Disconnected from user root 203.0.113.9 port 22")
            .await;
        assert_eq!(n.sent, 1);
    }

    #[test]
    fn deux_destinations_configurees() {
        let cfg: Config = toml::from_str(
            r#"
            [discord]
            webhook_url = "https://discord.com/api/webhooks/1/x"

            [slack]
            webhook_url = "https://hooks.slack.com/services/T0/B0/x"
            "#,
        )
        .unwrap();
        let n = Notifier::new(cfg, false).unwrap();
        assert_eq!(n.destinations(), ["discord", "slack"]);
    }

    #[test]
    fn slack_seul() {
        let cfg: Config =
            toml::from_str("[slack]\nwebhook_url = \"https://hooks.slack.com/services/T0/B0/x\"\n")
                .unwrap();
        let n = Notifier::new(cfg, false).unwrap();
        assert_eq!(n.destinations(), ["slack"]);
    }
}
