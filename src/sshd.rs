use std::net::IpAddr;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Accepted,
    Failed,
    InvalidUser,
    Closed,
}

impl Kind {
    pub fn is_failure(self) -> bool {
        matches!(self, Kind::Failed | Kind::InvalidUser)
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Accepted => "🔓 Connexion SSH acceptée",
            Kind::Failed => "⛔ Échec d'authentification",
            Kind::InvalidUser => "🚨 Compte inexistant",
            Kind::Closed => "👋 Déconnexion",
        }
    }

    pub fn color(self) -> u32 {
        match self {
            Kind::Accepted => 0x2ecc71,
            Kind::Failed => 0xe67e22,
            Kind::InvalidUser => 0xe74c3c,
            Kind::Closed => 0x95a5a6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub kind: Kind,
    pub user: String,
    pub ip: Option<IpAddr>,
    pub port: Option<u16>,
    pub method: Option<String>,
    pub key: Option<String>,
}

impl Event {
    pub fn ip_str(&self) -> String {
        match self.ip {
            Some(ip) => ip.to_string(),
            None => "?".into(),
        }
    }
}

// "Oct 13 19:51:02 vps sshd[1234]: <message>". Le groupe 1 est le programme,
// le 2 le message. journald en mode --output=cat ne met pas de préfixe.
static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\w{3}\s+\d+\s+[\d:]+|\S+T\S+)\s+\S+\s+([\w.-]+)(?:\[\d+\])?:\s*(.*)$").unwrap()
});

static ACCEPTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Accepted (\S+) for (\S+) from (\S+) port (\d+)(?: ssh2)?(?:: (.+))?").unwrap()
});

static FAILED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Failed (\S+) for (invalid user )?(\S+) from (\S+) port (\d+)").unwrap()
});

static INVALID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Invalid user (\S*) from (\S+)(?: port (\d+))?").unwrap());

static CLOSED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Disconnected from user (\S+) (\S+) port (\d+)").unwrap());

pub fn parse(line: &str) -> Option<Event> {
    let line = line.trim();
    let msg = match PREFIX.captures(line) {
        // sshd-session et sshd-auth depuis OpenSSH 9.8
        Some(c) if c[1].starts_with("sshd") => line[c.get(2)?.range()].trim(),
        Some(_) => return None,
        None => line,
    };

    // Un scanner qui raccroche avant de s'authentifier. L'échec lui-même est
    // déjà sur une autre ligne, inutile de compter ça deux fois.
    if msg.contains("[preauth]") || msg.contains("authenticating user") {
        return None;
    }

    if let Some(c) = ACCEPTED.captures(msg) {
        return Some(Event {
            kind: Kind::Accepted,
            user: c[2].into(),
            ip: ip(&c[3]),
            port: c[4].parse().ok(),
            method: Some(c[1].into()),
            key: c.get(5).map(|m| m.as_str().trim().into()),
        });
    }

    if let Some(c) = FAILED.captures(msg) {
        return Some(Event {
            kind: match c.get(2) {
                Some(_) => Kind::InvalidUser,
                None => Kind::Failed,
            },
            user: c[3].into(),
            ip: ip(&c[4]),
            port: c[5].parse().ok(),
            method: Some(c[1].into()),
            key: None,
        });
    }

    if let Some(c) = INVALID.captures(msg) {
        return Some(Event {
            kind: Kind::InvalidUser,
            user: if c[1].is_empty() {
                "(vide)".into()
            } else {
                c[1].into()
            },
            ip: ip(&c[2]),
            port: c.get(3).and_then(|m| m.as_str().parse().ok()),
            method: None,
            key: None,
        });
    }

    if let Some(c) = CLOSED.captures(msg) {
        return Some(Event {
            kind: Kind::Closed,
            user: c[1].into(),
            ip: ip(&c[2]),
            port: c[3].parse().ok(),
            method: None,
            key: None,
        });
    }

    None
}

fn ip(s: &str) -> Option<IpAddr> {
    s.strip_prefix("::ffff:").unwrap_or(s).parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Event {
        parse(line).unwrap()
    }

    #[test]
    fn accepted() {
        let e = p("Accepted publickey for root from 203.0.113.9 port 54321 ssh2: RSA SHA256:abc");
        assert_eq!(e.kind, Kind::Accepted);
        assert_eq!(e.user, "root");
        assert_eq!(e.ip_str(), "203.0.113.9");
        assert_eq!(e.port, Some(54321));
        assert_eq!(e.method.as_deref(), Some("publickey"));
        assert_eq!(e.key.as_deref(), Some("RSA SHA256:abc"));

        // préfixe syslog, et sshd-session depuis OpenSSH 9.8
        let e = p(
            "Oct 13 19:51:02 vps sshd[12]: Accepted password for thib from 1.2.3.4 port 22 ssh2",
        );
        assert_eq!(e.user, "thib");
        assert_eq!(e.method.as_deref(), Some("password"));
        let e = p("Oct 13 19:51:02 vps sshd-session[9]: Accepted publickey for ops from 1.2.3.4 port 22 ssh2: ED25519 SHA256:x");
        assert_eq!(e.user, "ops");
    }

    #[test]
    fn failed() {
        let e = p("Failed password for thib from 1.2.3.4 port 40112 ssh2");
        assert_eq!(e.kind, Kind::Failed);

        // compte inexistant: deux formulations, même verdict
        let e = p("Failed password for invalid user admin from 1.2.3.4 port 5 ssh2");
        assert_eq!(e.kind, Kind::InvalidUser);
        assert_eq!(e.user, "admin");
        let e = p("Invalid user oracle from 1.2.3.4 port 6");
        assert_eq!(e.kind, Kind::InvalidUser);
        assert_eq!(e.port, Some(6));
        assert_eq!(p("Invalid user  from 1.2.3.4 port 7").user, "(vide)");
    }

    #[test]
    fn closed() {
        let e = p("Disconnected from user root 203.0.113.9 port 54321");
        assert_eq!(e.kind, Kind::Closed);
        assert_eq!(e.user, "root");
    }

    #[test]
    fn ips() {
        assert_eq!(
            p("Accepted password for root from ::ffff:203.0.113.9 port 22 ssh2").ip_str(),
            "203.0.113.9"
        );
        assert_eq!(
            p("Accepted publickey for root from 2001:db8::1 port 22 ssh2").ip_str(),
            "2001:db8::1"
        );
    }

    #[test]
    fn noise() {
        for line in [
            "Connection closed by authenticating user root 1.2.3.4 port 1 [preauth]",
            "Received disconnect from 1.2.3.4 port 1:11: Bye Bye [preauth]",
            "Server listening on 0.0.0.0 port 22.",
            "pam_unix(sshd:session): session closed for user root",
            "Oct 13 19:51:02 vps sudo[42]: thib : TTY=pts/0 ; PWD=/root",
            "Oct 13 19:51:02 vps CRON[9]: pam_unix(cron:session): session opened for user root",
            "",
        ] {
            assert!(parse(line).is_none(), "devrait passer: {line}");
        }
    }
}
