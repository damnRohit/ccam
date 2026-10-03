# AGENTS.md

For an agent changing ccam. Operating ccam for a user is a different job: see `skills/ccam/SKILL.md`.

ccam swaps Claude Code's live login between saved accounts. A bug here logs a real person out of a real subscription, so every change is judged first by one question: **can this spend a refresh token twice, or lose the only working copy of one?**

## Where things are

| Path | Holds |
|---|---|
| `src/manager.rs` | Every account operation (save, sync, switch, refresh) and the typed lock order. Start here. |
| `src/presence.rs` | Which account each install sharing the vault (Windows, WSL) is live on |
| `src/login.rs` | `OAuthLogin` (parsed, redacted `Debug`), generations, `Identity` |
| `src/vault.rs` | Saved accounts on disk |
| `src/lock.rs` | `mkdir` locks compatible with Claude Code's proper-lockfile |
| `src/jsonio.rs` | Atomic JSON writes |
| `src/auto.rs` / `src/autorun.rs` | `ccam auto`: pure decision logic / the loop around it |
| `src/main.rs`, `src/help.rs` | CLI wiring and `--help` text |
| `tests/cli.rs` | End-to-end tests of the built binary |
| `docs/SPEC.md` | The design and invariants; `docs/research/` the evidence behind them |
| `scripts/` | Optional WSL shell helpers, separate from the Rust crate |

## Invariants

These are the contract. `docs/SPEC.md` §5 explains each one.

1. A saved login is never replaced by an older generation of the same identity.
2. A login is saved only under an identity it is confirmed to belong to. Renewals within one session are trusted; any other new login is confirmed through the profile endpoint first.
3. Before the live login is replaced, the account being left is up to date in the vault. Switching away from an unsaved, unreadable or unconfirmed login is refused.
4. Locks are taken in one order: vault, Claude Code's refresh locks, the global config lock. The types enforce it: a write takes its lock proof as an argument.
5. No network call while Claude Code's locks are held.
6. Every write is atomic (unique temp file, fsync, rename) and keeps existing permissions.
7. Unknown keys in Claude Code's files are preserved, in order. Only `claudeAiOauth` and `oauthAccount` are ever written.
8. An account live on another install is never switched to, renewed, or picked by `auto` here. The check and the presence record happen under the vault lock.

A change that needs to bend one of these needs the owner's agreement and a SPEC update in the same change.

## Secrets

Tokens and email addresses never appear in output, logs, test snapshots or commit messages. `OAuthLogin`'s `Debug` is redacted; keep it so. `switch.log` holds account names and events only.

## Tests

Tests run against a temporary config home with `CCAM_CREDENTIAL_STORE=file` and `CCAM_API_BASE` pointed at a fake server. They must never read or write the real `~/.claude` or macOS Keychain, and never touch the network. A new test follows the same pattern: copy the fixture of its neighbours (`fixture()` in `manager.rs`, `setup()` in `autorun.rs`, `Home` in `tests/cli.rs`).

Test behaviour at the public seams: `Manager` methods, `auto::decide`, `Runner::tick`, the CLI. A bug fix starts with a test that goes red on the bug.

## Verifying a change

Done means all of these pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo clippy --target x86_64-apple-darwin --all-targets --no-default-features -- -D warnings
```

The last line type-checks the macOS Keychain code, which no maintainer can run. Code under `#[cfg(unix)]` compiles only on Linux and macOS, so a change there also needs `cargo test` on Linux or in WSL. CI runs the full matrix.

When behaviour depends on Claude Code itself (in-place switching, hooks, token renewal), automated tests cannot prove it. Add a step to `docs/MANUAL-TESTS.md`.

## Undocumented ground

Claude Code's credentials layout, lock files, `refreshTokenExpiresAt` behaviour and the usage, profile and token endpoints are not public API. `docs/research/internals.md` records what was observed and from which Claude Code version. When you rely on something new from Claude Code, record it there with the version, and mark it *undocumented* in SPEC.

## Docs that must change with the code

- A new or changed command: `src/help.rs`, the README command table, and `skills/ccam/SKILL.md` when an agent would use it.
- A new error message: `docs/TROUBLESHOOTING.md`.
- A changed file layout or invariant: `docs/SPEC.md`.
