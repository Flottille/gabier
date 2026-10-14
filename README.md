# Gabier

Envoie une notification Discord et/ou Slack à chaque connexion SSH sur le
serveur.

<p align="center">
  <img src="https://gabier.nebulabs.org/media/how-it-works.gif"
       alt="Une connexion SSH acceptée devient une notification Discord en moins d'une seconde ; un scan de 43 tentatives n'en produit qu'une seule, résumée."
       width="800">
</p>

<p align="center">
  <a href="https://gabier.nebulabs.org">Site et documentation</a> ·
  <a href="https://gabier.nebulabs.org/install/">Installation</a> ·
  <a href="https://gabier.nebulabs.org/docs/throttling/">L'anti-spam</a>
</p>

```
🔓 Connexion SSH acceptée
Clé `ED25519 SHA256:T9mQ0kR1vZ8pL2xH7bN4...`
Utilisateur `root`   IP `203.0.113.42`
Méthode `publickey`  Port 51288
                                     vps-prod-1 • 09:03
```

Lit les logs de sshd (journald, ou `/var/log/auth.log` sur une distro sans
systemd), en extrait les connexions acceptées, les échecs et les tentatives
sur des comptes inexistants.

## L'anti-spam

C'est le seul vrai piège. Un VPS avec le port 22 ouvert prend des milliers
de tentatives par jour : tout notifier noie le salon, et tu rates la seule
notification qui compte.

Donc par IP : les `burst` premiers échecs partent tout de suite, les suivants
sont comptés et résumés en un message à la fin de la fenêtre.

```
🚨 Rafale de tentatives SSH
43 tentatives refusées depuis 185.243.96.107.
Comptes visés  admin, oracle, postgres, test, ubuntu
```

Les connexions **réussies** ne sont jamais regroupées.

## Installation

Sur le serveur :

```bash
cargo build --release
sudo ./install.sh
sudo nano /etc/gabier/config.toml   # webhook + nom du serveur
sudo -u gabier gabier check         # valide la config, joint Discord
sudo -u gabier gabier test          # un message dans le salon
sudo systemctl start gabier
```

Les webhooks :

- **Discord** — *Paramètres du salon → Intégrations → Webhooks → Nouveau
  webhook → Copier l'URL*
- **Slack** — [api.slack.com/apps](https://api.slack.com/apps) → ton app →
  *Incoming Webhooks* → *Add New Webhook to Workspace*

Au moins une des deux sections `[discord]` / `[slack]` doit être présente.
Si les deux sont là, chaque notification part aux deux — et si l'une tombe,
l'autre passe quand même.

Pour garder les secrets hors du fichier de config :

```bash
sudo tee /etc/gabier/env >/dev/null <<'EOF'
GABIER_DISCORD_WEBHOOK=https://discord.com/api/webhooks/...
GABIER_SLACK_WEBHOOK=https://hooks.slack.com/services/...
EOF
sudo chmod 600 /etc/gabier/env
```

Sans Rust sur le serveur, on construit un binaire Linux statique ailleurs :

```bash
docker run --rm -v "$PWD:/app" -w /app rust:1-slim sh -c \
  "rustup target add x86_64-unknown-linux-musl && \
   cargo build --release --target x86_64-unknown-linux-musl"
scp target/x86_64-unknown-linux-musl/release/gabier vps:/tmp/
# puis sur le VPS: sudo ./install.sh /tmp/gabier
```

## Commandes

```
gabier                surveille (= gabier run)
gabier test           envoie un message de test
gabier check          valide la config et joint Discord, sans publier
gabier replay f.log   rejoue un fichier de log
```

`--config <fichier>` partout ; sinon `./config.toml` puis
`/etc/gabier/config.toml`. `--dry-run` affiche au lieu d'envoyer :

```bash
gabier replay tests/fixtures/auth.log --dry-run --config config.example.toml
```

## Config

Voir [`config.example.toml`](config.example.toml). Les deux destinations ont
exactement la même forme, seule la syntaxe des pings change :

```toml
[discord]
webhook_url = "https://discord.com/api/webhooks/..."
ping = "<@123456789>"          # ou "<@&123456789>" pour un rôle
ping_on_accepted = true

[slack]
webhook_url = "https://hooks.slack.com/services/..."
ping = "<!channel>"            # ou "<@U0123ABCD>", "<!subteam^S0123ABCD>"
ping_on_failure = true
```

Sur une distro sans systemd :

```toml
[source]
kind = "file"
path = "/var/log/auth.log"
```

La lecture suit les rotations, pas besoin de redémarrer après un logrotate.

## Dev

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

```
sshd.rs     ligne de log → Event
watch.rs    journald / fichier / stdin → canal de lignes
notify.rs   filtre, déduplique, regroupe, décide d'envoyer
hook.rs     Msg + POST avec réessais, partagé par les destinations
discord.rs  payload Discord (embed)
slack.rs    payload Slack (attachment coloré)
geo.rs      géoloc optionnelle
```

Pour ajouter Telegram ou ntfy : un `payload()`, et une variante de `Sink`
dans `notify.rs`. Le retry, les limites de taille et le bridage des pings
sont déjà dans `hook.rs`.

## À savoir

- Le service tourne sous un compte dédié sans shell, pas en root. Il lui
  faut juste le groupe `systemd-journal`.
- Côté Discord, `allowed_mentions` est bridé : un pseudo dans une ligne de
  log ne peut pas déclencher un `@everyone`. Côté Slack, le ping n'est mis
  dans `text` que s'il est explicitement configuré.
- sshd autorise 3 essais de mot de passe par connexion (même port source).
  La déduplication les replie en une seule notification — voulu pour le
  bruit, mais le compteur de rafale sous-estime d'autant.
- Gabier observe, il ne bannit rien. `fail2ban` reste utile à côté.
