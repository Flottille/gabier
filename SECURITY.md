# Security Policy

Gabier runs on a server, reads the authentication log, and holds a credential
that can post into a team's chat. Reports are welcome and taken seriously.

## Supported versions

| Version | Supported |
| ------- | --------- |
| `main`  | yes       |
| 0.1.x   | yes       |

The project is pre-1.0 and there is no backport branch: fixes ship in a new
patch release from `main`. If you run a build older than the latest tag, please
reproduce on the latest one before reporting.

## Reporting a vulnerability

**Do not open a public issue.**

Use GitHub's private reporting on this repository: **Security → Advisories →
Report a vulnerability**
([direct link](https://github.com/Flottille/gabier/security/advisories/new)).
Only the maintainers can read it, and the fix can be prepared in a private fork
and published as an advisory from the same place.

Useful in a report:

- what an attacker gets out of it, and what they need to start with — a log line
  of their choosing, a local account on the server, the webhook URL, read access
  to `/etc/gabier/`;
- the affected version or commit;
- a reproduction. `gabier replay <file> --dry-run` is usually enough, and it
  sends nothing — a crafted log file plus that command line is the ideal
  report;
- the config shape, with the webhook URL removed.

What to expect: an acknowledgement within 7 days, and an honest assessment of
severity and timeline rather than silence. This is a small project maintained in
spare time, not a vendor with an on-call rota. You will be credited in the
advisory and the release notes unless you ask not to be.

Please don't run scans or exploit attempts against infrastructure that is not
yours — against `gabier.nebulabs.org` included. Everything worth testing here
can be tested locally with a log file.

## In scope

The things worth looking at, roughly in order of how much they would matter:

- **Webhook leakage.** The Discord or Slack webhook URL is a credential: anyone
  holding it can post into the channel. Anything that puts it where it should
  not be — process output, `tracing` logs, an error message, a notification
  body, a file readable by another local user, a request to a third party.
- **Mention clamping bypass.** Discord payloads pin
  `allowed_mentions.parse` to `[]` unless a ping is explicitly configured,
  so a username or a hostname appearing inside a log line must never be able to
  produce an `@everyone`, `@here`, or a role ping. On Slack the ping is only
  placed in `text` when configured. A log line that defeats either of those is a
  vulnerability, not a formatting bug.
- **Crafted log lines.** The parser's input is attacker-chosen by definition —
  anyone who can reach port 22 can write into the log Gabier reads. A line that
  panics the daemon is a denial of service on *notifications*: with
  `panic = "abort"` the process dies, and an attacker who can trigger it on
  demand can keep the alerting down while doing something else. Unbounded
  memory or CPU growth from one line, or from a crafted sequence, counts too.
- **Notification suppression.** Any way to make a real accepted login go
  unreported — abusing deduplication, the burst window, or the grouping logic to
  hide a successful connection in a flood. Successful logins are deliberately
  never grouped; a bypass of that is in scope.
- **Privilege and file handling.** The service account has no shell, no
  capabilities, and `ProtectSystem=strict`. A path traversal, a symlink
  follow, a world-readable config, or a permission `install.sh` gets wrong that
  would let a local user read the webhook or escalate.
- **Data sent off the box.** With `geoip = true` each source IP is sent to
  `ipwho.is`; that is documented and off by default. Anything sent anywhere else,
  or sent with `geoip = false`, is a bug worth reporting.

## Out of scope

- Gabier does not ban, block, or rate-limit anything. "Attacks continued after
  the notification" is the design, not a flaw — pair it with `fail2ban`.
- The volume of SSH attempts a public server receives, and the alert fatigue it
  causes. That is what throttling is for; report a throttling *bypass* instead.
- Outages, rate limits, or data handling at Discord, Slack, or `ipwho.is`.
- A server configured against the documentation: webhook in a world-readable
  file, service run as root, `/etc/gabier/env` left at default permissions.
  Please do report documentation that *leads* someone there.
- Attacks that assume root on the monitored server. Root can read the config and
  edit the log; there is nothing to defend at that point.
- Dependency advisories with no reachable path in Gabier. A plain `cargo audit`
  dump is not a report; say which call path reaches it. (A routine dependency
  bump is fine as a normal pull request.)

## For operators

If a webhook URL has leaked, rotate it — delete the webhook in Discord or Slack
and create a new one. Editing the commit, the issue, or the log does not revoke
it.

The defaults are the hardened path, and they are worth keeping:

- the service runs as `gabier`, a dedicated account with no shell, whose only
  privilege is membership of `systemd-journal`;
- `systemd/gabier.service` drops all capabilities, blocks new privileges, and
  confines the filesystem — don't relax it to fix a permission problem, fix the
  permission;
- keep the secret in `/etc/gabier/env` with `chmod 600` rather than in
  `config.toml`;
- `geoip` stays `false` unless you accept that every source IP goes to a third
  party;
- `gabier check` validates the config and reaches the destination without
  posting, which is the safe command to run after any change.
