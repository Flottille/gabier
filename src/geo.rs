use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use serde::Deserialize;

/// Cache borné: un scanner peut défiler des milliers d'IP en une nuit.
const MAX: usize = 512;

#[derive(Deserialize)]
struct Reply {
    success: Option<bool>,
    country: Option<String>,
    city: Option<String>,
    connection: Option<Conn>,
    flag: Option<Flag>,
}

#[derive(Deserialize)]
struct Conn {
    isp: Option<String>,
}

#[derive(Deserialize)]
struct Flag {
    emoji: Option<String>,
}

pub struct Geo {
    http: reqwest::Client,
    cache: HashMap<IpAddr, Option<String>>,
}

impl Geo {
    pub fn new() -> Self {
        Geo {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(4))
                .build()
                .unwrap_or_default(),
            cache: HashMap::new(),
        }
    }

    /// "🇫🇷 Paris, France — OVH". Jamais d'erreur: une notification sans
    /// géoloc vaut mieux qu'une notification perdue.
    pub async fn lookup(&mut self, ip: IpAddr) -> Option<String> {
        if private(ip) {
            return None;
        }
        if let Some(hit) = self.cache.get(&ip) {
            return hit.clone();
        }
        if self.cache.len() >= MAX {
            self.cache.clear();
        }
        let found = self.fetch(ip).await;
        self.cache.insert(ip, found.clone());
        found
    }

    async fn fetch(&self, ip: IpAddr) -> Option<String> {
        let r = self
            .http
            .get(format!("https://ipwho.is/{ip}"))
            .send()
            .await
            .ok()?;
        let r: Reply = r.json().await.ok()?;
        if r.success != Some(true) {
            return None;
        }

        let mut out = String::new();
        if let Some(f) = r.flag.and_then(|f| f.emoji) {
            out.push_str(&f);
            out.push(' ');
        }
        let place: Vec<String> = [r.city, r.country].into_iter().flatten().collect();
        out.push_str(&place.join(", "));
        if let Some(isp) = r.connection.and_then(|c| c.isp) {
            out.push_str(" — ");
            out.push_str(&isp);
        }

        let out = out.trim().to_string();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                // 100.64/10, le CGNAT des opérateurs
                || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments()[0];
            // fc00::/7 et fe80::/10
            v6.is_loopback() || v6.is_unspecified() || s & 0xfe00 == 0xfc00 || s & 0xffc0 == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prive_vs_public() {
        for s in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.20",
            "172.16.4.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
        ] {
            assert!(private(s.parse().unwrap()), "{s}");
        }
        for s in ["8.8.8.8", "203.0.113.9", "2001:db8::1"] {
            assert!(!private(s.parse().unwrap()), "{s}");
        }
    }
}
