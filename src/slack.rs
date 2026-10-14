use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};

use crate::config::{Hook, Service};
use crate::hook::{self, cut, Msg};

pub struct Slack {
    http: reqwest::Client,
    url: String,
    cfg: Hook,
}

impl Slack {
    pub fn new(cfg: &Hook) -> Result<Self> {
        Ok(Slack {
            http: hook::client(10)?,
            url: cfg.url(Service::Slack)?,
            cfg: cfg.clone(),
        })
    }

    pub async fn send(&self, msg: &Msg) -> Result<()> {
        hook::post(&self.http, &self.url, &self.payload(msg), "slack").await
    }

    fn payload(&self, msg: &Msg) -> Value {
        // Slack n'a pas d'équivalent des embeds: la barre de couleur passe
        // par une attachment, et mrkdwn_in réactive le markdown dedans.
        let fields: Vec<Value> = msg
            .fields
            .iter()
            .take(10)
            .map(|(k, v)| json!({ "title": k, "value": cut(v, 2000), "short": true }))
            .collect();

        let mut att = json!({
            "color": format!("#{:06x}", msg.color),
            "title": cut(&msg.title, 300),
            "fields": fields,
            "footer": cut(&msg.footer, 300),
            "ts": Utc::now().timestamp(),
            "mrkdwn_in": ["text", "fields"],
            // Affiché par les clients qui ne savent pas rendre l'attachment
            // (notifications mobiles, par exemple).
            "fallback": cut(&msg.title, 300),
        });
        if let Some(b) = &msg.body {
            att["text"] = json!(cut(b, 3000));
        }

        let mut p = json!({ "attachments": [att] });
        if let Some(u) = &self.cfg.username {
            p["username"] = json!(u);
        }
        if let Some(ping) = msg.ping.and_then(|k| self.cfg.ping_for(k)) {
            p["text"] = json!(ping);
        }
        p
    }
}

/// Slack ne répond pas à un GET sur un webhook entrant: un 400
/// "invalid_payload" prouve que l'URL existe, un 404 "no_service" qu'elle est
/// morte. C'est le seul moyen de vérifier sans poster dans le salon.
pub async fn probe(cfg: &Hook) -> Result<()> {
    let s = Slack::new(cfg)?;
    let r = s
        .http
        .get(&s.url)
        .send()
        .await
        .context("appel du webhook")?;
    let code = r.status();
    let body = r.text().await.unwrap_or_default();

    if code == 404 || body.trim() == "no_service" {
        bail!("slack: webhook inconnu ou révoqué — recrée-le dans l'app Slack");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sshd::Kind;

    fn hook() -> Hook {
        Hook {
            webhook_url: "https://hooks.slack.com/services/T0/B0/xxx".into(),
            username: None,
            ping: None,
            ping_on_accepted: false,
            ping_on_failure: false,
        }
    }

    fn msg() -> Msg {
        Msg {
            title: "🔓 Connexion".into(),
            color: 0x2ecc71,
            body: Some("Clé `abc`".into()),
            fields: vec![("Utilisateur".into(), "`root`".into())],
            footer: "vps".into(),
            ping: Some(Kind::Accepted),
        }
    }

    #[test]
    fn attachment_colore() {
        let p = Slack::new(&hook()).unwrap().payload(&msg());
        let att = &p["attachments"][0];
        assert_eq!(att["color"], "#2ecc71");
        assert_eq!(att["title"], "🔓 Connexion");
        assert_eq!(att["text"], "Clé `abc`");
        assert_eq!(att["fields"][0]["title"], "Utilisateur");
        assert_eq!(att["fields"][0]["short"], true);
        assert_eq!(att["fallback"], "🔓 Connexion");
        assert!(p["text"].is_null(), "pas de ping par défaut");
    }

    #[test]
    fn ping_dans_le_texte() {
        // Un ping doit être hors de l'attachment, sinon Slack ne notifie pas.
        let mut h = hook();
        h.ping = Some("<!channel>".into());
        h.ping_on_accepted = true;
        let p = Slack::new(&h).unwrap().payload(&msg());
        assert_eq!(p["text"], "<!channel>");

        let mut m = msg();
        m.ping = None;
        assert!(Slack::new(&h).unwrap().payload(&m)["text"].is_null());
    }

    #[test]
    fn limites_slack() {
        let mut m = msg();
        m.fields = (0..20)
            .map(|i| (format!("f{i}"), "x".repeat(4000)))
            .collect();
        let p = Slack::new(&hook()).unwrap().payload(&m);
        let fields = p["attachments"][0]["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 10);
        assert_eq!(fields[0]["value"].as_str().unwrap().chars().count(), 2000);
    }
}
