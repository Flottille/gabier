## What this changes

<!-- And why. If it closes an issue: Closes #123 -->

## How you tested it

<!--
No server needed — the fixture and dry-run cover most cases:

  cargo run -- replay tests/fixtures/auth.log --dry-run --config config.example.toml

If you tested against a real sshd, say which distribution and OpenSSH version,
and paste the (redacted) notification it produced.
-->

## Checklist

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --all-targets -- -D warnings` — clean
- [ ] `cargo test --all` — green
- [ ] The example config still parses:
      `cargo run --release -- replay tests/fixtures/auth.log --dry-run --config config.example.toml`
- [ ] No webhook URL, token, real IP, username, hostname or key fingerprint in
      the diff — fixtures included
- [ ] README / `config.example.toml` updated if a config key or a command changed
- [ ] No runtime string translated in passing (see
      [Localisation](https://github.com/Flottille/gabier/blob/main/CONTRIBUTING.md#localisation))

## Anything else

<!-- Trade-offs, things you were unsure about, what you'd like reviewed first. -->
