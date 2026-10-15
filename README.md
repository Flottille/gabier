# Gabier

Sends a Discord and/or Slack notification on every SSH login to your server.

<p align="center">
  <img src="https://gabier.nebulabs.org/media/how-it-works.gif"
       alt="An accepted SSH login becomes a Discord notification in under a second; a 43-attempt scan produces a single summary."
       width="800">
</p>

<p align="center">
  <a href="https://gabier.nebulabs.org">Site and documentation</a> ·
  <a href="https://gabier.nebulabs.org/install/">Install</a> ·
  <a href="https://gabier.nebulabs.org/docs/throttling/">Throttling</a>
</p>

```
🔓 Connexion SSH acceptée
Clé `ED25519 SHA256:T9mQ0kR1vZ8pL2xH7bN4...`
Utilisateur `root`   IP `203.0.113.42`
Méthode `publickey`  Port 51288
                                     vps-prod-1 • 09:03
```

> The daemon's own output is still French — the block above is what it
> actually posts. See [Localisation](#localisation).

Reads the sshd logs (journald, or `/var/log/auth.log` on a distribution
without systemd) and pulls out accepted logins, failures, and attempts on
accounts that do not exist.

## Throttling

This is the only real trap. A VPS with port 22 open takes thousands of
attempts a day: notify all of them and the channel is drowned, which means
you miss the one notification that matters.

So, per IP: the first `burst` failures go out immediately, the rest are
counted and summarised in a single message at the end of the window.

```
🚨 Rafale de tentatives SSH
43 tentatives refusées depuis 185.243.96.107.
Comptes visés  admin, oracle, postgres, test, ubuntu
```

**Successful** logins are never grouped.

## Install

On the server:

```bash
cargo build --release
sudo ./install.sh
sudo nano /etc/gabier/config.toml   # webhook + server name
sudo -u gabier gabier check         # validates the config, reaches Discord
sudo -u gabier gabier test          # one message in the channel
sudo systemctl start gabier
```

The webhooks:

- **Discord** — *Channel settings → Integrations → Webhooks → New webhook →
  Copy the URL*
- **Slack** — [api.slack.com/apps](https://api.slack.com/apps) → your app →
  *Incoming Webhooks* → *Add New Webhook to Workspace*

At least one of the `[discord]` / `[slack]` sections must be present. With
both, every notification goes to both — and if one is down, the other still
gets through.

To keep the secrets out of the config file:

```bash
sudo tee /etc/gabier/env >/dev/null <<'EOF'
GABIER_DISCORD_WEBHOOK=https://discord.com/api/webhooks/...
GABIER_SLACK_WEBHOOK=https://hooks.slack.com/services/...
EOF
sudo chmod 600 /etc/gabier/env
```

Without Rust on the server, build a static Linux binary elsewhere:

```bash
docker run --rm -v "$PWD:/app" -w /app rust:1-slim sh -c \
  "rustup target add x86_64-unknown-linux-musl && \
   cargo build --release --target x86_64-unknown-linux-musl"
scp target/x86_64-unknown-linux-musl/release/gabier vps:/tmp/
# then on the VPS: sudo ./install.sh /tmp/gabier
```

## Commands

```
gabier                watch (= gabier run)
gabier test           send a test message
gabier check          validate the config and reach Discord, without posting
gabier replay f.log   replay a log file
```

`--config <file>` works everywhere; otherwise `./config.toml`, then
`/etc/gabier/config.toml`. `--dry-run` prints instead of sending:

```bash
gabier replay tests/fixtures/auth.log --dry-run --config config.example.toml
```

## Config

See [`config.example.toml`](config.example.toml). Both destinations take
exactly the same shape; only the mention syntax differs:

```toml
[discord]
webhook_url = "https://discord.com/api/webhooks/..."
ping = "<@123456789>"          # or "<@&123456789>" for a role
ping_on_accepted = true

[slack]
webhook_url = "https://hooks.slack.com/services/..."
ping = "<!channel>"            # or "<@U0123ABCD>", "<!subteam^S0123ABCD>"
ping_on_failure = true
```

On a distribution without systemd:

```toml
[source]
kind = "file"
path = "/var/log/auth.log"
```

Reading follows rotations; there is nothing to restart after a logrotate.

## Dev

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

```
sshd.rs     log line → Event
watch.rs    journald / file / stdin → a channel of lines
notify.rs   filters, deduplicates, groups, decides whether to send
hook.rs     Msg + POST with retries, shared by both destinations
discord.rs  Discord payload (embed)
slack.rs    Slack payload (coloured attachment)
geo.rs      optional geolocation
```

To add Telegram or ntfy: one `payload()`, and a `Sink` variant in
`notify.rs`. Retries, size limits and ping clamping already live in
`hook.rs`.

## Localisation

The documentation is in English; the binary's runtime strings are still
French (`Gabier est dans la hune`, `Notification de test envoyée`, the
Discord embed labels). They live in `main.rs`, `discord.rs` and `slack.rs`.
Translating them is a breaking change for anyone already reading those
messages, so it has not been done silently — the samples above are verbatim
output, not stale documentation.

## Worth knowing

- The service runs under a dedicated account with no shell, not as root. All
  it needs is the `systemd-journal` group.
- On Discord, `allowed_mentions` is clamped: a username inside a log line
  cannot trigger an `@everyone`. On Slack, the ping is only placed in `text`
  when one is explicitly configured.
- sshd allows 3 password attempts per connection (same source port).
  Deduplication folds them into a single notification — deliberate, for the
  noise — but the burst counter under-counts by the same amount.
- Gabier observes, it never bans. `fail2ban` is still useful alongside it.

## Contributing

Issues and patches are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) covers what
fits, what CI runs, and how to add a destination — the code is laid out for it.
Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

Found a vulnerability? [SECURITY.md](SECURITY.md) — privately, not in an issue.
