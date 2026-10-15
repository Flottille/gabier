# Contributing

Thanks for looking. Gabier is small on purpose: it reads sshd logs and posts a
notification. Patches that keep it that way are the easy ones to merge.

## What fits, what doesn't

**Fits**

- A new destination — Telegram, ntfy, Matrix, a plain webhook. This is the most
  useful contribution, and the code is laid out for it (see below).
- sshd log lines that are not parsed yet, or are parsed wrong, on a distribution
  or OpenSSH version I don't have.
- Throttling and deduplication bugs: a notification that should have been
  grouped and wasn't, or the reverse.
- Documentation, including fixing my English.

**Doesn't fit**

- Banning, blocking, rate-limiting at the firewall. *Gabier observes, it never
  bans.* `fail2ban` already does that job well and runs happily alongside.
- Anything that needs a database, a daemon of its own, or a port open.
- Translating the binary's runtime strings on its own — see
  [Localisation](#localisation).

If you are unsure, open an issue before writing the code. Saying "no thanks" to
a finished patch is the thing I would least like to do.

## Getting set up

You do not need a server, or even an SSH daemon. The repository ships a log
fixture, and `replay --dry-run` prints notifications instead of sending them:

```bash
cargo build
cargo run -- replay tests/fixtures/auth.log --dry-run --config config.example.toml
```

Rust stable, pinned by `rust-toolchain.toml`. No system dependency to install:
TLS is rustls with embedded roots, deliberately, so there is no OpenSSL to
match on a VPS.

## Before you open a pull request

Run what CI runs, in this order:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --release && ./target/release/gabier replay tests/fixtures/auth.log \
  --dry-run --config config.example.toml
```

Clippy warnings are errors in CI, so `-D warnings` locally saves a round trip.
The last command is there to catch an example config that no longer parses —
it is the file everyone copies to `/etc/gabier/config.toml`.

## Where things live

```
sshd.rs     log line → Event
watch.rs    journald / file / stdin → a channel of lines
notify.rs   filters, deduplicates, groups, decides whether to send
hook.rs     Msg + POST with retries, shared by both destinations
discord.rs  Discord payload (embed)
slack.rs    Slack payload (coloured attachment)
geo.rs      optional geolocation
config.rs   config.toml + GABIER_* environment variables
main.rs     CLI: run / test / check / replay
```

### Adding a destination

One `payload()` function, and a `Sink` variant in `notify.rs`. Retries, size
limits and ping clamping already live in `hook.rs` — use them rather than
writing a second HTTP client. `discord.rs` and `slack.rs` are each short enough
to read in full before you start; copy the shape of whichever is closer.

Make the new section optional, like `[discord]` and `[slack]`: an operator with
one destination must not have to configure the others. Add it to
`config.example.toml`, commented out, and to the README's Config section.

### Adding or fixing a log pattern

Add the real line to `tests/fixtures/auth.log` and a unit test in `sshd.rs`
asserting the `Event` it produces. **Redact it first**: replace the IP with one
from a documentation range (`203.0.113.x`, `198.51.100.x`), shorten the key
fingerprint, and change the username if it is not a generic one. A fixture is a
public file forever.

Say in the pull request which OpenSSH version and distribution produced the
line, so the next person knows what the test is actually pinning.

## Secrets, logs, and what not to paste

Everything about this project involves data you would rather not publish.

- Never commit a webhook URL. A Discord or Slack webhook URL is a credential on
  its own — anyone holding it can post to the channel. If one ever lands in a
  commit, in an issue, or in CI output, rotate it: delete the webhook and make a
  new one. Scrubbing the text is not enough.
- Redact IPs, usernames, hostnames and key fingerprints in issues and pull
  requests, including in pasted service logs.
- Tests must not reach the network. `--dry-run` and the fixtures exist so they
  don't have to.

## Commits

Short imperative subject, no prefix tags, no `feat:`/`fix:` convention here —
`git log --oneline` shows the style:

```
Point the manifest at the repository and the site
Add the MIT licence, and stop ignoring the test fixtures
```

Put the *why* in the body when it isn't obvious from the diff; the code already
says what. Keep unrelated changes in separate commits. A rebase on `main` is
preferred over a merge commit, but I'm not going to reject a patch over it.

## Localisation

The documentation is English. The binary's runtime strings are still French
(`Gabier est dans la hune`, `Notification de test envoyée`, the Discord embed
labels) and live in `main.rs`, `discord.rs` and `slack.rs`.

Translating them changes what every existing operator sees in their channel, and
what their own alerting greps for, so it is a breaking change, not a cleanup.
A patch that translates one file in passing will be asked to drop that part.
If you want the binary in English — or properly localised — open an issue and
let's do it in one go, for one release, with the release notes to match.

## Licence

Gabier is MIT. By opening a pull request you agree your contribution ships
under the same licence. There is no CLA.

## Reporting a vulnerability

Not here, and not in an issue — see [SECURITY.md](SECURITY.md).
