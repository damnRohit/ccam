# ccam specification

`ccam` (Claude Code Account Manager) keeps several Claude subscription logins on one machine and switches Claude Code between them without `/logout` and `/login`.

Status: v0.1, updated 2026-10-03. Facts about Claude Code internals come from `docs/research/internals.md`; anything marked *undocumented* can change with a Claude Code release. This document describes the code as built; when they disagree, the code is wrong or this file is stale, and either is a bug.

## 1. Goals and non-goals

Goals
- Save any number of logged-in accounts under user-chosen names.
- Switch the default Claude Code config to a saved account, in place or by restarting the session.
- Never lose a login to refresh-token rotation.
- Show 5-hour, 7-day and per-model usage for every saved account.
- Windows, macOS and Linux. One static binary.

Non-goals
- Per-account `CLAUDE_CONFIG_DIR` profiles. That model is documented and avoids token copying, but gives each account its own history. `ccam` exists for the shared-history case. See 9.1.
- API keys, Console logins, Bedrock or Vertex.

## 2. Vocabulary

| Term | Meaning |
|---|---|
| Config home | `$CLAUDE_CONFIG_DIR` if set, else `~/.claude` |
| Global config | `$CLAUDE_CONFIG_DIR/.claude.json` if set, else `~/.claude.json`; a legacy `<config home>/.config.json` wins when present |
| Live login | the `claudeAiOauth` object Claude Code uses now, in `<config home>/.credentials.json` (Windows, Linux, macOS fallback) or the macOS Keychain |
| Live identity | `oauthAccount` in the global config |
| Live state | logged out, unreadable (a login or identity exists but does not parse), or present. A login whose access and refresh tokens are both blank is logged out: Claude Code leaves that shape behind when a renewal is refused, and it holds nothing to save or lose |
| Install | one Claude Code config home on one platform; Windows and WSL on one PC are two installs that can share a vault (section 9.6) |
| Account | a saved login and identity under a name |
| Vault | `<config home>/ccam/accounts/`, one directory per account |
| Generation | one issued login of an account; a later generation has a later `expiresAt` |
| Session | the lifetime of one `/login`; renewals within it share `refreshTokenExpiresAt` (*observed, undocumented*) |

Identity is `accountUuid` plus `organizationUuid` (`None` and `Some` are different identities). `emailAddress` is display data and is never printed by default or logged.

## 3. Data

```text
AccountName   ^[A-Za-z0-9][A-Za-z0-9._@-]{0,63}$, compared case-insensitively
UnixMillis    a point in time, so expiries, clocks and buffers cannot be mixed up as plain integers
OAuthLogin    parsed from claudeAiOauth: accessToken, refreshToken and expiresAt are required, so a login
              without them is "unreadable" and is never saved or replaced; unknown fields are kept verbatim
Identity      { account_uuid, organization_uuid: Option }
Account       { name, identity, oauth_account (verbatim), login }
```

Vault layout:

```text
<config home>/ccam/accounts/
  <name>/login.json      {"claudeAiOauth": {...}}   owner-only on Unix
  <name>/account.json    the oauthAccount object verbatim
  .vault.lock            ccam's own lock directory
  .live/<key>.json       which account each install sharing the vault is live on (section 9.6)
  .auto/<key>/           per-install `ccam auto` lock, stop file, status and hook signals
  switch.log             append-only, no secrets
```

`mcpOAuth` and every other key in the live credentials file are machine state and are never copied into the vault or overwritten by a switch. A vault entry that fails to load is reported by `list` and skipped; it never blocks other accounts.

## 4. Commands

| Command | Behaviour |
|---|---|
| `list` | Accounts, `*` marking the one matching the live identity. Reports unloadable entries and an unsaved or unreadable live login. |
| `save <name>` | Saves the live login under `name`. Confirms over the network that the token belongs to the live identity; refuses when Anthropic says otherwise, and warns when it could not check. Refuses when the identity is already saved under another name, when `name` holds a different account, or when the saved copy is a newer generation. |
| `rename <old> <new>` | Renames an account. Case-only renames are allowed. |
| `remove <name>` | Deletes an account from the vault. The live login is untouched. |
| `to <name> [--restart] [--open [session]] [--continue] [--force]` | Switches to `name` (section 5). From a normal terminal, `--open` then runs `claude` (plain, or `--resume <session>`), `--continue` runs `claude --continue`; both are refused inside Claude Code. Refused when another install is live on `name`; `--force` first checks this install could switch, then moves the other install to the free account with the most room by running its ccam (section 9.6). |
| `start [--refresh]` | One minimal `claude -p` per saved account without a running 5-hour window (section 8a). |
| `usage [--refresh] [--json]` | Usage per account (section 7). |
| `sync [--quiet]` | Copies the live login into its matching account when it is a newer generation. |
| `setup` / `uninstall` | Adds or removes ccam's hooks in `<config home>/settings.json` (section 6). |
| `import-legacy` | Hidden. One-time conversion of directories written by the author's PowerShell prototype (section 9.5). |
| `hook <event> [--config-dir]` | Internal, called by the hooks. Always exits 0. |
| `resume ...` | Internal, run in the new tab by `--restart`. |

Exit codes: 0 success; 1 the request cannot be done as asked (bad name, unknown account, wrong state); 2 files, locks or the credential store failed; 3 Anthropic's endpoints failed or the login could not be confirmed.

## 5. Switching and syncing

Invariants
1. A saved login is never replaced by an older generation of the same identity. A live login that is older than its saved copy is a conflict: `sync` reports it and `to` refuses.
2. A login is only ever saved under an identity it is confirmed to belong to. Renewals within the same session (same `refreshTokenExpiresAt`) are trusted; any other new login is confirmed through the profile endpoint before it is saved, and `to` refuses until that has happened.
3. Before the live login is replaced, the account being left is up to date in the vault. Switching away from an unsaved, unreadable or unconfirmed login is refused.
4. Lock order, always: `.vault.lock`, then Claude Code's `.oauth_refresh.lock`, then `<config home>.lock`, then `<global config>.lock`. The order is enforced by the type system: `CredentialLocks` can only be built from a `VaultLock` and `SwitchLocks` only from a `CredentialLocks`, and every write takes the lock it needs as an argument, so a write without the right lock does not compile. Claude Code's locks are `mkdir` directories, stale after 60 s (credentials) or 10 s (config); ccam touches held locks every 3 s and gives up after 9 s. Each write re-checks that the lock is still the one this process created, so a lock taken over while the machine slept is noticed. Timing relationships (heartbeat well inside the stale window, and waiting past it after a kill) are checked at compile time.
5. No network call happens while Claude Code's locks are held. Claude Code retries a held lock only a few times before failing its own refresh.
6. Every file write is atomic: a uniquely named temp file, fsync, rename. Existing file permissions are preserved.
7. Unknown keys in the credentials file and global config are preserved, in order.
8. An account live on another install sharing the vault is never switched to, renewed by `usage --refresh` / `start --refresh`, or picked by `auto`. `to` checks this before taking locks (to fail fast) and again under the vault lock, and writes this install's presence record under that same lock, so two installs switching at once cannot both take one account.

`sync` reads under the credential locks and, when a login needs confirming, releases them, asks the profile endpoint, then re-locks and re-checks before writing. The foreground `SessionStart` hook and the implicit sync before read-only commands run offline and leave unconfirmed logins for a later trigger.

`to <name>` runs a full sync first, then under all four locks re-reads the vault and live state, saves a renewed login of the account being left, writes the target identity, then the target login. If the login write fails the identity is restored.

Session handling after an in-place switch: on Windows and Linux a running session reads the new login on its next request; on macOS within about 30 s (*undocumented*, checked per release in `docs/MANUAL-TESTS.md`).

`--restart` (Windows Terminal only): the old tab pre-checks the switch, opens a new tab running `ccam resume`, waits for it to signal ready (20 s, else nothing is switched), waits until 5 s after launch, takes Claude Code's refresh locks so no token refresh is in flight, terminates the old Claude Code and its plain shell tab, and releases the locks once it has exited. The new tab waits for the old process to exit (60 s), switches (waiting out a lock the killed process may have left), and runs `claude --continue`. The config dir travels as an argument because a tab in an existing window inherits that window's environment.

`usage --refresh` renews a saved account only under the vault lock and only when neither the live identity nor the live refresh token matches it, then saves the new generation before anything else uses it. Because `to` also takes the vault lock, a switch can never install a refresh token a refresh is about to spend.

## 6. Hooks

`setup` installs, idempotently:

| Event | Matcher | Runs | Action |
|---|---|---|---|
| `SessionStart` | `startup\|resume` | foreground | offline sync; prints `{"hookSpecificOutput": {"hookEventName": "SessionStart", "watchPaths": ["<credentials file>"]}}` |
| `FileChanged` | none | background | sync when `file_path` is the credentials file |
| `Notification` | `auth_success` | background | sync (covers macOS, where no file changes) |

Handlers are exec-form (`command` = the ccam binary, `args` = `["hook", <event>, ...]`), recognised by that shape, so reinstalling replaces them and uninstalling never touches other hooks. When `CLAUDE_CONFIG_DIR` is set at setup time it is baked into `args`, because Claude Code can strip it from hook environments. Hooks always exit 0; failures go to `switch.log`.

## 7. Usage

- Endpoints (*undocumented*, the ones Claude Code itself calls): `GET https://api.anthropic.com/api/oauth/usage` and `.../api/oauth/profile` with `Authorization: Bearer` and `anthropic-beta: oauth-2025-04-20`; `POST https://platform.claude.com/v1/oauth/token` with `grant_type=refresh_token` and Claude Code's public client id.
- Fields read: `five_hour{utilization,resets_at}`, `seven_day{...}`, `limits[]{scope.model.display_name, percent, resets_at}`.
- Accounts are fetched concurrently, 10 s timeout each. The live account uses the live token and is never refreshed by ccam. Other accounts with an expired token show `stale` unless `--refresh`.
- Cards share one width. Bars are green below 70 %, yellow below 90 %, red otherwise. Colour follows `NO_COLOR` and TTY detection. `--json` prints `[{name, current, plan, status: ok|stale|error, windows|message}]`.

## 8b. Automatic switching (`ccam auto`)

Originally a non-goal because it is what the Consumer Terms' automation and "bypassing ... protective measures" clauses are most easily read against. Added at the owner's request on 2026-09-24 with that risk documented in the README; `--notify` offers the same watcher without automatic switches. Research: `docs/research/auto-switch.md`.

Signals: `ccam setup --auto` adds an async `Stop` hook (turn ended) and an async `StopFailure:rate_limit` hook (limit hit); each touches a file under `<vault>/.auto/<key>/`, where `<key>` names this install, so a limit hit in WSL never moves Windows. The loop (`autorun.rs`) ticks every 5 s and reads only file mtimes unless a check is due.

Checks of the live account (`auto::Schedule`): quiet intervals 30/15/5/2 min for levels under 50 / 80 / the arm level (threshold minus 8, at least 50) / above it; after a finished turn 10/5/2/1 min; a limit signal forces one check. A 429 blocks checks for 15 min, doubling to 75 min. Other accounts are read only once the live account is armed, one per tick, each at most every 10 min; a switch target must have been read within 5 min.

Decision (`auto::decide`, pure): below the threshold nothing happens. Over it, the target is the eligible account (5-hour use under threshold minus 20, weekly under threshold, at least 20 points more headroom than the live account, not the account just left) with the most headroom in 10-point bands, then the soonest weekly and 5-hour resets; a window past its reset counts as empty. Auto mode switches when a turn ends, immediately on a hit limit, or after 10 min without a finished turn; switches are 15 min apart (a hit limit overrides this) and at most one per account per rolling five hours. Notify mode announces the target once per crossing and re-arms after dropping 10 points. With no eligible account it reports the account that reopens first and waits.

Accounts another install is live on are left out of the candidates and never renewed. Process, all under `<vault>/.auto/<key>/`: one `mkdir` lock `lock` (a second copy on the same install refuses to start); `ccam auto stop` writes `stop`, which the loop honours between ticks, never mid-switch; `status.json` backs `ccam auto status`. Notifications: terminal line with bell, desktop (Windows toast with a looping alarm, also from WSL through `powershell.exe`; `notify-send`; macOS `osascript`), optional ntfy-style URL. All best effort.

## 8a. Starting usage windows

`start` iterates saved accounts. For each: skip when the usage endpoint reports a running `five_hour` window; obtain a usable access token (the live account's live token, never renewed here; a saved account's token, renewed under the vault lock only with `--refresh`); run `claude -p "hi" --model haiku --system-prompt "Reply with one word." --tools "" --strict-mcp-config --disable-slash-commands --max-turns 1 --no-session-persistence --output-format json` in a temporary config dir holding only `{"hasCompletedOnboarding":true}` and an empty working dir, with `CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_DISABLE_CLAUDE_MDS`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC` set and every `ANTHROPIC_*`, `CLAUDE*`, `AWS_*`, Vertex, Bedrock and Foundry variable removed; then re-read usage (up to 4 tries, 8 s apart, the endpoint lags) and report `started` only when a window appears. Measured on 2026-09-19: 271 input and 54 to 97 output tokens, about $0.0007, window appears within about 10 s, live login and config unchanged. An env token is never refreshed by Claude Code, so nothing rotates; `--bare` cannot be used because it ignores OAuth tokens.

## 8. Platforms

| | Windows | Linux | macOS |
|---|---|---|---|
| Live login | file | file | Keychain `Claude Code-credentials[-<sha256(CLAUDE_CONFIG_DIR)[..8]>]`, account `$USER`; file when the file exists and no item does. A locked Keychain is an error, never "logged out". |
| Keychain write | | | `security -i` with the hex value on stdin; argv only when too large for the interactive buffer. Verified by reading the item back, because `security -i` exits 0 on failure. |
| Restart | Windows Terminal tab | prints `claude --continue` | prints `claude --continue` |

`CCAM_CREDENTIAL_STORE=file` forces the file store; tests set it so they can never touch a real Keychain. `CCAM_API_BASE` redirects all three endpoints to one host for tests.

## 9. Decisions

### 9.1 In-place swap
Per-account `CLAUDE_CONFIG_DIR` is race-free but splits history, settings and plugins, and cannot change a running session. In-place swap keeps one history at the cost of locks and generation tracking.

### 9.2 Rust
A single static binary suits a tool that runs inside hooks. TypeScript would need Node, which Claude Code's native installer does not ship.

### 9.3 Crates
`clap`, `serde` + `serde_json` (`preserve_order`), `ureq` with rustls, `thiserror`, `sha2` + `hex` (Keychain name), `jiff`, `filetime` (lock heartbeats), `windows-sys` (process control, file identity). Dev: `tempfile`, `assert_cmd`, `predicates`.

### 9.4 Sync triggers
No hook fires on `/login`. `FileChanged` on the credentials file fires on login, logout and every rotation, never per reply. macOS uses `auth_success`.

### 9.5 The prototype
The PowerShell prototype (`cs`) stores accounts in `<config home>/accounts/`. ccam keeps its own vault in `<config home>/ccam/accounts/` and never writes to the prototype's folder: `import-legacy` only reads it, so trying ccam cannot damage the prototype's accounts (an end-to-end test checks the folder stays byte-for-byte identical). The live login is shared by both tools, since it is Claude Code's. Running both tools on the same accounts is unsupported: each spends refresh tokens the other's copies still hold.

### 9.6 Installs sharing a vault
Windows and WSL keep separate live logins, so two vaults would go stale on every renewal (each spends tokens the other's copy holds). They share one vault instead: WSL's `<config home>/ccam` links to the Windows one over DrvFs. That makes one account live on both installs the remaining hazard, handled by presence records:

- Each install writes `<vault>/.live/<key>.json` with the live identity, its platform (`windows`, `wsl` plus distro, `other`) and the path of its ccam. `<key>` is a short hash of OS, distro and credentials path, so two distros with the same home path stay distinct.
- A record changes only on a definite state: present writes the identity, logged out removes it, unreadable or a read error leaves it, so a login caught mid-write never clears it.
- Writing a record also removes records with the same platform and ccam path under another key: they are this install's own, left from an older key, and would otherwise block its account forever.
- `--force` reaches the other install with `wsl.exe -d <distro> -- <ccam>` (from Windows or WSL) or by translating `C:\...\ccam.exe` to `/mnt/c/...` (from WSL). Other combinations are not reachable and report the record file to delete.
- From WSL, a sharing violation on a vault file arrives as `EACCES` and is retried as on Windows; a refused `chmod` (`EPERM`, a DrvFs mount without the user's uid) keeps the temp file's creation mode instead of failing the write.

Known gap: ccam's own vault lock decides staleness from the lock's mtime against local wall-clock time, and the two kernels' clocks can drift apart after host sleep.

## 10. Testing

Automated (`cargo test`, no network, never the real `~/.claude`):
1. Vault and manager operations against a temporary config home and a fake OAuth client.
2. The generation and confirmation rules, including a mid-`/login` file mismatch and offline behaviour.
3. Locks: contention, heartbeat, stale takeover, takeover detection.
4. Atomic writes under concurrent writers; permission preservation on Unix.
5. Hook installer and handler.
6. API client against a local fake HTTP server; usage collection and rendering.
7. The built binary end to end, with the file store forced and the API base pointed at an unroutable address.

Manual, per Claude Code release: `docs/MANUAL-TESTS.md`.

CI: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test` on Windows, Ubuntu and macOS; `cargo check --locked` on the minimum Rust version.
