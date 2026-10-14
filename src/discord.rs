use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};

use crate::config::{Hook, Service};
use crate::hook::{self, cut, Msg};

pub struct Discord {
    http: reqwest::Client,
    url: String,
    cfg: Hook,
}

impl Discord {
    pub fn new(cfg: &Hook) -> Result<Self> {
        Ok(Discord {
            http: hook::client(10)?,
            url: cfg.url(Service::Discord)?,
            cfg: cfg.clone(),
        })
    }

    pub async fn send(&self, msg: &Msg) -> Result<()> {
        hook::post(&self.http, &self.url, &self.payload(msg), "discord").await
    }

    fn payload(&self, msg: &Msg) -> Value {
        let fields: Vec<Value> = msg
            .fields
            .iter()
            .take(25)
            .map(|(k, v)| json!({ "name": k, "value": cut(v, 1024), "inline": true }))
            .collect();

        let mut p = json!({
            "embeds": [{
                "title": cut(&msg.title, 256),
                "color": msg.color,
                "fields": fields,
                "footer": { "text": cut(&msg.footer, 2048) },
                "timestamp": Utc::now().to_rfc3339(),
            }],
            // Sans ping explicite, on coupe tout: un pseudo dans une ligne de
            // log ne doit pas pouvoir déclencher un @everyone.
            "allowed_mentions": { "parse": [] },
        });

        if let Some(b) = &msg.body {
            p["embeds"][0]["description"] = json!(cut(b, 4096));
        }
        if let Some(u) = &self.cfg.username {
            p["username"] = json!(u);
        }
        if let Some(ping) = msg.ping.and_then(|k| self.cfg.ping_for(k)) {
            p["content"] = json!(ping);
            p["allowed_mentions"] = json!({ "parse": ["users", "roles"] });
        }
        p
    }
}

/// GET sur le webhook: valide l'URL sans rien publier.
pub async fn probe(cfg: &Hook) -> Result<()> {
    let d = Discord::new(cfg)?;
    let r = d
        .http
        .get(&d.url)
        .send()
        .await
        .context("appel du webhook")?;
    if !r.status().is_success() {
        bail!(
            "discord répond {} — récupère l'URL dans Paramètres du salon > Intégrations",
            r.status()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sshd::Kind;

    fn hook() -> Hook {
        Hook {
            webhook_url: "https://discord.com/api/webhooks/1/x".into(),
            username: Some("Gabier".into()),
            ping: None,
            ping_on_accepted: false,
            ping_on_failure: false,
        }
    }

    fn msg() -> Msg {
        Msg {
            title: "t".into(),
            color: 0x2ecc71,
            body: None,
            fields: vec![],
            footer: "vps".into(),
            ping: Some(Kind::Accepted),
        }
    }

    #[test]
    fn pings_coupes_par_defaut() {
        let p = Discord::new(&hook()).unwrap().payload(&msg());
        assert_eq!(p["allowed_mentions"]["parse"], json!([]));
        assert_eq!(p["username"], "Gabier");
        assert!(p["content"].is_null());
    }

    #[test]
    fn ping_quand_demande() {
        let mut h = hook();
        h.ping = Some("<@42>".into());
        h.ping_on_accepted = true;
        let p = Discord::new(&h).unwrap().payload(&msg());
        assert_eq!(p["content"], "<@42>");
        assert_eq!(p["allowed_mentions"]["parse"], json!(["users", "roles"]));

        // un message de service ne ping jamais
        let mut m = msg();
        m.ping = None;
        let p = Discord::new(&h).unwrap().payload(&m);
        assert!(p["content"].is_null());
    }

    #[test]
    fn limites_discord() {
        let mut m = msg();
        m.fields = (0..40)
            .map(|i| (format!("f{i}"), "x".repeat(2000)))
            .collect();
        let p = Discord::new(&hook()).unwrap().payload(&m);
        assert_eq!(p["embeds"][0]["fields"].as_array().unwrap().len(), 25);
        let v = p["embeds"][0]["fields"][0]["value"].as_str().unwrap();
        assert_eq!(v.chars().count(), 1024);
    }
}
