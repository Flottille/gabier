use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::header::RETRY_AFTER;
use reqwest::Response;
use serde_json::Value;
use tracing::warn;

use crate::sshd::Kind;

const TRIES: u32 = 4;

/// Un message prêt à partir. Chaque destination en fait ce qu'elle veut.
pub struct Msg {
    pub title: String,
    pub color: u32,
    pub body: Option<String>,
    pub fields: Vec<(String, String)>,
    pub footer: String,
    /// Le type d'évènement, pour que chaque destination décide seule si elle
    /// ping et qui. `None` = message de service, jamais de ping.
    pub ping: Option<Kind>,
}

impl Msg {
    pub fn text(&self) -> String {
        let mut s = format!("{} [{}]", self.title, self.footer);
        if let Some(b) = &self.body {
            s += &format!("\n  {b}");
        }
        for (k, v) in &self.fields {
            s += &format!("\n  {k}: {v}");
        }
        s
    }
}

pub fn client(timeout: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout))
        .build()
        .context("client HTTP")
}

/// POST avec réessais. `who` sert juste aux messages d'erreur.
pub async fn post(http: &reqwest::Client, url: &str, payload: &Value, who: &str) -> Result<()> {
    for n in 1..=TRIES {
        let wait = match http.post(url).json(payload).send().await {
            Ok(r) if r.status().is_success() => return Ok(()),

            // Discord et Slack annoncent tous deux l'attente en en-tête.
            Ok(r) if r.status() == 429 => retry_after(&r).unwrap_or(Duration::from_secs(1)),

            // 400/401/403/404: payload refusé, ou webhook révoqué. Réessayer
            // n'y changera rien.
            Ok(r) => {
                let code = r.status();
                let body = r.text().await.unwrap_or_default();
                if code.is_client_error() {
                    bail!("{who} refuse ({code}): {}", body.trim());
                }
                warn!("{who} a répondu {code}: {}", body.trim());
                backoff(n)
            }

            Err(e) if n == TRIES => return Err(e).context(format!("envoi vers {who}")),
            Err(e) => {
                warn!("{who}: {e}");
                backoff(n)
            }
        };

        if n == TRIES {
            bail!("{who} injoignable après {TRIES} tentatives");
        }
        tokio::time::sleep(wait).await;
    }
    unreachable!()
}

fn backoff(n: u32) -> Duration {
    Duration::from_secs(2u64.pow(n))
}

fn retry_after(r: &Response) -> Option<Duration> {
    let secs: f64 = r
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs_f64(secs.clamp(0.1, 60.0)))
}

/// Discord et Slack comptent en caractères, pas en octets.
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max - 1).chain(['…']).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn troncature() {
        assert_eq!(cut("abc", 10), "abc");
        assert_eq!(cut("abcdef", 3), "ab…");
        // pas de caractère multi-octets coupé en deux
        assert_eq!(cut("ééééé", 2).chars().count(), 2);
    }
}
