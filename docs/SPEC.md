# ccam specification

`ccam` (Claude Code Account Manager) keeps several Claude subscription logins on one machine and switches Claude Code between them without `/logout` and `/login`.

Status: v0.1, updated 2026-10-30. Facts about Claude Code internals come from `docs/research/internals.md`; anything marked *undocumented* can change with a Claude Code release. This document describes the code as built; when they disagree, the code is wrong or this file is stale, and either is a bug.

## 1. Goals and non-goals

Goals
- Save any number of logged-in accounts under user-chosen names.
- Switch the default Claude Code config to a saved account, in place or by restarting the session.
- Never lose a login to refresh-token rotation.
- Show 5-hour, 7-day and per-model usage for every saved account.
- Windows, macOS and Linux. One static binary.
- Keep chosen terminals on chosen accounts (lanes, `ccam run`) while others use the main login, sharing history and memory (section 9.7).

Non-goals
- Free-standing per-account `CLAUDE_CONFIG_DIR` profiles that keep their own history. Lanes are config dirs too, but share history and memory with the main one and check their login out of the vault. See 9.1 and 9.7.
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
| Fresh login | a login made by `ccam login` in a throwaway config dir; a new session, so it is confirmed through the profile endpoint like any other (section 9.9) |
| Instance | a T3 Code provider entry; a lane has exactly one, `ccam-lane-<id>` (section 9.8) |

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

While `ccam login` runs, its throwaway config dir is `<config home>/ccam/login/<pid>-<time>/`, outside `accounts/` and not linked to it. It is deleted on every exit path, together with Claude Code's `<dir>.lock` beside it (section 9.9). The run holds the lock directory `<config home>/ccam/login.lock` throughout.

`mcpOAuth` and every other key in the live credentials file are machine state and are never copied into the vault or overwritten by a switch. A vault entry that fails to load is reported by `list` and skipped; it never blocks other accounts.

## 4. Commands

| Command | Behaviour |
|---|---|
| `list` | Accounts, `*` marking the one matching the live identity. Reports unloadable entries and an unsaved or unreadable live login. |
| `save <name>` | Saves the live login under `name`. Confirms over the network that the token belongs to the live identity; refuses when Anthropic says otherwise, and warns when it could not check. Refuses when the identity is already saved under another name, when `name` holds a different account, or when the saved copy is a newer generation. |
| `login <name>` | Signs in with Claude Code's own `claude auth login` in a throwaway config dir and saves the result under `name` (section 9.9). The main login, lanes and open sessions are untouched. A new login is saved as a new account. For a saved `name`, replaces that account's saved login, but only while the account is live nowhere. Refused when the sign-in was to an account saved under another name, when `name` holds a different account, and when the login cannot be read. |
| `rename <old> <new>` | Renames an account. Case-only renames are allowed. |
| `remove <name>` | Deletes an account from the vault. The live login is untouched. |
| `to <name> [--restart] [--open [session]] [--continue] [--force]` | Switches to `name` (section 5). From a normal terminal, `--open` then runs `claude` (plain, or `--resume <session>`), `--continue` runs `claude --continue`; both are refused inside Claude Code. Refused when another install is live on `name`; `--force` first checks this install could switch, then moves the other install to the free account with the most room by running its ccam (section 9.6). |
| `run <name> [--lane <id>] [claude args...]`, `run --lane <id> [-- claude args...]` | Opens Claude Code in this terminal with `CLAUDE_CONFIG_DIR` set to a lane: the lane already holding `name`, else `--lane <id>` (created when missing), else the lowest free number. Checks `name` out into the lane when it holds none (section 9.7). Refused inside Claude Code, from inside a lane, when `name` is the main login, when `name` is in another lane than `--lane`, for a lane holding another account, and below Claude Code 2.1.296. After checking out, it brings the T3 Code instances up to date, silently (section 9.8). |
| `lane <name> [--lane <id>]` | Creates a lane (or reuses the one holding `name`, else `--lane <id>`, else the lowest free number) and checks `name` out into it without opening Claude Code, picking and refusing as `run` does: from inside a lane, when `name` is the main login, when `name` is in another lane than `--lane`, and for a lane holding another account. It does not check the Claude Code version (T3 runs its own) and may run inside Claude Code. Then prints `Lane <id> holds '<name>'.` and brings the T3 Code instances up to date (section 9.8). |
| `move <lane> <name> [--yes]` | Switches a lane to `name` (section 9.7) and relabels its T3 Code instance. Asks first; without a terminal it needs `--yes`. |
| `release <lane> [--force] [--delete]` | Checks a lane's account back into the vault and logs the lane out (section 9.7), and disables its T3 Code instance. Refused while `claude agents --json` lists sessions in it, unless `--force`. `--delete` also removes the lane folder and its instance. |
| `prune` | Removes lane directories that hold no account and have no running sessions, and their T3 Code instances. |
| `sessions` | The main login and each lane: account held and running sessions, from `claude agents --json` per config dir. |
| `start [--refresh]` | One minimal `claude -p` per saved account without a running 5-hour window (section 8a). |
| `usage [--refresh] [--json] [--email] [--info]` | Usage per account (section 7). `--email` adds each account's `oauthAccount.emailAddress` from the vault, in the box title and as `email` in JSON. `--info` implies `--email` and adds account, billing and guest pass details (section 7.1). Only `--email` and `--info` print an email address, and only when asked; emails, names and billing details are never logged. |
| `sync [--quiet]` | Copies the live login into its matching account when it is a newer generation. |
| `setup` / `uninstall` | Adds or removes ccam's hooks in `<config home>/settings.json` (section 6). |
| `import-legacy` | Hidden. One-time conversion of directories written by the author's PowerShell prototype (section 9.5). |
| `hook <event> [--config-dir]` | Internal, called by the hooks. Always exits 0. |
| `resume ...` | Internal, run in the new tab by `--restart`. |

Exit codes: 0 success; 1 the request cannot be done as asked (bad name, unknown account, wrong state); 2 files, locks or the credential store failed; 3 Anthropic's endpoints failed or the login could not be confirmed.

## 5. Switching and syncing

Invariants
1. A saved login is never replaced by an older generation of the same identity. A live login that is older than its saved copy is a conflict: `sync` reports it and `to` refuses. One exception: a confirmed fresh login (`ccam login`, section 9.9) may replace a saved login of the same account, even one that looks newer, but only while that account is live nowhere. A fresh login is a separate token family, and the saved copy is not in use by anything.
2. A login is only ever saved under an identity it is confirmed to belong to. Renewals within the same session (same `refreshTokenExpiresAt`) are trusted; any other new login is confirmed through the profile endpoint before it is saved, and `to` refuses until that has happened.
3. Before the live login is replaced, the account being left is up to date in the vault. Switching away from an unsaved, unreadable or unconfirmed login is refused.
4. Lock order, always: `.vault.lock`, then Claude Code's `.oauth_refresh.lock`, then `<config home>.lock`, then `<global config>.lock`. The order is enforced by the type system: `CredentialLocks` can only be built from a `VaultLock` and `SwitchLocks` only from a `CredentialLocks`, and every write takes the lock it needs as an argument, so a write without the right lock does not compile. Claude Code's locks are `mkdir` directories, stale after 60 s (credentials) or 10 s (config); ccam touches held locks every 3 s and gives up after 9 s. Each write re-checks that the lock is still the one this process created, so a lock taken over while the machine slept is noticed. Timing relationships (heartbeat well inside the stale window, and waiting past it after a kill) are checked at compile time. `ccam login` also holds its own `login.lock` (section 9.9), taken before all of these and released after them; nothing that holds them takes it.
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
- Cards share one width. Bars are green below 70 %, yellow below 90 %, red otherwise. Colour follows `NO_COLOR` and TTY detection. `--json` prints `[{name, current, plan, plan_label, status: ok|stale|error, windows|message}]`.
- The plan label comes from the saved login's `subscriptionType` and `rateLimitTier`: `Pro`, `Max 5x` (`default_claude_max_5x`), `Max 20x` (`default_claude_max_20x`), `Max`, `Team`, `Enterprise`, else the raw type. It shows in the box border and as `plan_label`.
- `extra_usage{is_enabled, used_credits, monthly_limit, currency, decimal_places}` is read from the usage response ccam already fetches. While `is_enabled` is true the card shows `Extra usage  <used> / <limit>` (`unlimited` when `monthly_limit` is null), amounts in minor units of `currency` (two places when `decimal_places` is absent). A currency that is not a three-letter ISO code is dropped (`null` in JSON), and the amounts then print without one. The same object is `extra_usage` in JSON.

### 7.1 Account details (`usage --info`)

- Only with `--info`. Per account with a usable token, one `GET /api/oauth/profile` with that account's own token (*undocumented*). Fields read by name: `account.display_name`, `account.full_name`, `account.created_at`, `organization.name`, `organization_type`, `rate_limit_tier`, `billing_type`, `subscription_status`, `subscription_created_at`, `claude_code_trial_ends_at`, `seat_tier`, `has_extra_usage_enabled`. Nothing else is parsed, so ids, the email, the invoice link and the referral code never reach output.
- Guest passes, Pro and Max only. First the cache Claude Code keeps in the main `~/.claude.json`, `passesEligibilityCache[<organizationUuid>]`, used when its `timestamp` is under 24 hours old (*undocumented*). Otherwise one `GET /api/oauth/organizations/<org>/referral/eligibility?campaign=claude_code_guest_pass` with the bearer token and an `x-organization-uuid` header (*undocumented*). Read: `eligible`, `remaining_passes`, `limit`, `referrer_reward`. Never the code or links. ccam only reads the cache and never writes it (invariant 7).
- A failed request shows `info unavailable: <reason>` or `guest passes unavailable: <reason>` and the card still renders. No renewal date is shown: the billing period, next charge, monthly or annual billing and cancel date are not in any OAuth response (claude.ai only, `docs/research/billing.md`).
- JSON: each account gets an `info` object with the profile fields flat (or `unavailable`), `extra_usage_disabled_reason`, `credit`, `guest_passes` and `raw`: the usage keys ccam does not interpret, keeping only numbers, booleans, nulls and ISO 8601 timestamps. Other strings, keys not shaped like `[a-z0-9_]{1,48}` and containers left empty are dropped, and `raw` never appears in Debug output. Enum-like profile fields (`organization_type`, `rate_limit_tier`, `billing_type`, `subscription_status`, `seat_tier`, the extra-usage `disabled_reason`) are kept as returned only when they match `^[a-z0-9_]{1,48}$`, else `unrecognised`. The credit line is always labelled `Credit`. The organization id must be a canonical UUID before it goes into the eligibility URL or header.
- Nothing from `--info` is logged.

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

### 9.7 Lanes
An in-place switch moves every session on the install, and a session that changes account mid-conversation loses its prompt cache, which is isolated per organization. A lane is a config home under `<main config home>/ccam/lanes/<id>/` that holds one account; `ccam run` starts Claude Code with `CLAUDE_CONFIG_DIR` set to it. `<id>` is a number from 1 (no leading zeros) or a name in account-name syntax that is not all digits, compared without case. A lane is an install in the sense of 9.6, on the same machine:

- Its `ccam/accounts` links to the main vault (a junction on Windows, a symlink elsewhere), so locks, records and accounts are shared. Its record carries `"lane": "<id>"`, and lane and main share platform and ccam path, so the stale-record rule of 9.6 also compares `lane`.
- Checking out (`run`) and `move` are an ordinary `to <name>` against the lane's paths, so invariants 1 to 8 hold unchanged: the account must not be live on any other install, and the lane's record keeps the main login, other lanes, `usage --refresh`, `start --refresh` and `auto` off it. Rewriting the lane's credentials moves every session of that lane on its next request and no other session. When the lane already holds `name`, `run` syncs instead.
- `projects`, `file-history`, `plans`, `plugins`, `skills`, `commands`, `agents` and `output-styles` link to the main config home, so transcripts, `/resume` and `/rewind` work across lanes. `history.jsonl` (prompt history), `tasks` and `session-env` stay per lane. `CLAUDE.md`, `keybindings.json` and `settings.json` are copied from the main config home on every `run` and `lane`, and at the start of every session in the lane (section 9.8); the copied settings get ccam's hooks with `--config-dir <lane>`, so renewals in the lane reach the vault. The lane's global config is copied once, without `oauthAccount`.
- Auto memory stays the main login's. Through the `projects` link, Claude Code asks before every memory read and write and no rule can approve it (anthropics/claude-code#98044, reproduced on Windows 2.1.296, `docs/research/sessions.md` 2.2). So `run` launches with `--settings <lane>/ccam/settings/<slug>.json`, which sets `autoMemoryDirectory` to the real `<main>/projects/<slug>/memory` and allows `Read(//<that dir>/**)` so listing it does not ask either. `<slug>` is Claude Code's folder name for the repository root (`git rev-parse --git-common-dir`, else the working directory) with every non-alphanumeric character replaced by `-`. Names over 200 characters are matched against existing folders; with no single match, memory is not redirected and the run is logged.
- Links that already exist are kept. Removing a lane unlinks its shared junctions or symlinks before deleting the lane directory, so main history and the vault survive. `claude purge` run inside a lane does reach the main history.
- A lane is empty only when its live state is `LoggedOut` and it has no running sessions. Automatic allocation and `prune` never treat an unsaved identity or unreadable login as empty. `sessions` distinguishes `no account`, `unsaved login`, and `login unreadable`.
- Before `prune` or `release --delete` removes a lane, `Manager::remove_lane` takes the main vault lock, then that lane's refresh locks and global config lock, and rechecks that the login is still absent. Session listing runs before those locks, never while they are held. Deletion takes the typed lock proof. The vault lock uses the main path, so unlinking the lane's vault junction does not strand the lock. A login installed after the initial observation prevents deletion.
- `release` runs a full sync, then under all four locks requires the vault to hold exactly the live login before it clears `claudeAiOauth` (keeping other keys) and `oauthAccount`, and updates the record. A session still open in the lane loses its login, which is why it is refused while sessions run, unless `--force`.
- The account a lane holds is refused by `to` with the lane's `move` and `release` as fixes; `--force` does not move lanes.
- Every lane event is in `switch.log`, prefixed `[lane <id>]`: switches, syncs, releases, each `run` (folder, account, Claude Code version, the names of the flags passed to Claude Code, settings file; never a flag's value or a positional argument, which can be a prompt or a file name), Claude Code's exit, and each session start its hook sees.

Lanes rely on `CLAUDE_CONFIG_DIR` moving the credentials file, the global config and the refresh locks together (section 2), on Claude Code 2.1.280+ not deleting a junctioned folder in cleanup, and on 2.1.296's fix for sessions sharing a config dir (*undocumented*; `docs/MANUAL-TESTS.md` 8). Remote Control in main-login sessions can drop when a lane logs in (anthropics/claude-code#88855).

### 9.8 T3 Code instances
T3 Code runs Claude Code through the Agent SDK, so `ccam run` cannot sit in front of it (*undocumented*; `docs/research/t3code.md`, read from T3 0.1.5). T3's unit is the instance, a provider entry in `settings.json`, and a Claude instance can name the config dir its threads run in. So each lane gets one instance, and what ccam does to a lane reaches T3 with no change on T3's side.

- **Instance id.** T3's id rule is `^[a-zA-Z][a-zA-Z0-9_-]*$`, at most 64 characters (*undocumented*). A lane id made only of ASCII letters, digits and `-`, short enough to fit, is the instance `ccam-lane-<id>` as spelled. Any other lane id (with `.`, `_` or `@`, or too long) is sanitised, with every other character replaced by `_`, shortened if needed, and ends in `_` plus the first 8 hex characters of the SHA-256 of the lowercased lane id. Clean ids never contain `_`, so the two forms cannot meet, and the hash keeps lanes that sanitise or shorten alike apart.
- **Label and home.** The label is `Claude · lane <id> · <account>`, or `Claude · lane <id>` when the lane holds no account. `config.homePath` is the lane's folder.
- **What ccam owns.** `driver` (`claudeAgent`), `displayName`, `enabled` and `config.homePath` of `ccam-lane-*` entries in `providerInstances`, and nothing else. A lane is enabled only when its live login is a saved account, so a released lane is disabled. Entries whose lane is gone are removed. Every other key in the file, other instances and other keys inside a `ccam-lane-*` entry included, is kept as it is, in order. ccam never touches an entry whose id lacks the `ccam-lane-` prefix. Secrets are never written: T3 keeps sensitive values in its own store.
- **Where.** `$T3CODE_HOME/userdata/settings.json` (an empty `T3CODE_HOME` counts as unset), else `~/.t3/userdata/settings.json`. When the `userdata` folder does not exist, T3 is not installed. `lane` then prints `T3 Code not found; skipped adding the lane to T3.` and succeeds; the other commands skip the step without a message. With no lanes and no `providerInstances`, the file is not created.
- **When.** After `lane`, `run`, `move`, `release` (also `--delete`) and `prune`. `move`, `release` and `prune` sync even when they fail part way, since they may have changed lanes; `lane` and `run` sync only once the lane holds its account. The file is read, reconciled and written atomically, and only when something changed. A file that is not a JSON object, whose `providerInstances` is not one, or that does not parse, is an error and is never overwritten. After a write, ccam reads the file back and redoes the edit if it did not stick; the second write is the last, and a third mismatch is the error `T3 Code kept overwriting the lane instances; try again`. T3 re-reads the file before its own saves, but a save landing between its read and its rename can drop ccam's edit, which is what the read-back covers. T3 watches the file and loads a new instance within about a second, without a restart (*undocumented*).
- **Failure is a warning.** A sync failure prints `ccam: could not update T3 Code: <error>` on stderr and logs `t3: <error>`; it never fails the command, because the lane change before it has already happened, and the next lane command retries. A successful write is logged as `t3: updated`.
- **Output.** `lane` prints `Lane <id> holds '<account>'.` and then `T3 Code: lane <id> is in the model picker as '<label>'.`, or the not-found note, or nothing more after a warning.
- **Invariants.** The T3 file holds no login, so invariants 1 to 8 do not apply to it.

How T3 uses an instance (*undocumented*, from the 0.1.5 bundle, not tested end to end):
- Threads on different instances run on different config dirs, so on different accounts at the same time.
- A lane is its own continuation group for T3. Moving a thread to another instance, including another lane or the main one, starts a fresh native Claude session with a history T3 builds, at most 16,000 tokens by default. It never resumes the shared transcript, although `projects` is linked. `move` keeps the conversation instead: the thread stays on its instance, and only the login under it changes, at the cost of one prompt-cache rebuild.
- A child agent started with `delegate_task` or `create_threads` without a target gets its parent's instance, so it stays in the lane. With only a driver kind named, T3 can pick another healthy Claude instance, which is another account. Claude Code's own in-session subagents share the parent's config dir. Codex and Antigravity threads use their own logins.
- The SDK session sets no `settingSources`, so the lane's `settings.json` and ccam's hooks load as in `run`. T3 never calls `ccam run`, so the lane's `SessionStart` hook refreshes its copies of `settings.json`, `CLAUDE.md` and `keybindings.json` from the main config home, keeping ccam's hooks with `--config-dir <lane>`. The main config home is derived from the lane's path. `run` and `lane` do the same refresh. A file is written only when its content changed (a copy of `CLAUDE.md` or `keybindings.json` replaced like every other write, invariant 6; settings through the same edit as `setup`), and a main file that is missing leaves the lane's copy as it is. The hook logs `[lane <id>] settings refreshed` or `[lane <id>] settings not refreshed: <error>` and goes on to sync the login either way. An edit to the main files reaches the lane at its next session, not one already running.
- Accepted limit: ccam reads T3's `settings.json`, edits its own entries and replaces the file atomically. T3 re-reads the file before each of its own saves, so T3 never overwrites ccam's edit with a stale copy. The reverse is not covered: a T3 settings change saved in the milliseconds between ccam's read and its rename is lost. T3 offers no lock or transactional update to prevent it. ccam writes only when a lane entry changes, which keeps the window rare.
- Memory cannot be redirected as `run` does: T3 reads `--settings` out of an instance's launch arguments itself. T3 threads run in full-access mode (`bypassPermissions`), which the Claude Code docs say skips the check behind anthropics/claude-code#98044, so memory writes through the `projects` link are expected not to ask. Not yet tested; step 1 of `docs/MANUAL-TESTS.md` 10 checks it.

### 9.9 Fresh login
`/login` is the only other way to add an account. It replaces the main login and moves every open session, and the account being left must be saved first (invariant 3). `ccam login <name>` adds an account without any of that. Research: `docs/research/login.md`.

1. One sign-in at a time per config home. `ccam login` takes the `mkdir` lock `<config home>/ccam/login.lock` (outside `login/`, so the sweep below never removes it) and holds it until its directory is deleted. Its timing is the vault lock's (heartbeat 3 s, stale after 60 s), but a second `ccam login` waits only 1 s, since a sign-in can take minutes: it fails with `another ccam login is signing in; finish or cancel it first` (exit 1) before touching anything. A crashed run's lock goes stale after 60 s and is taken over. Directories left by an earlier run under `<config home>/ccam/login/` are then deleted. A crash can leave one holding live tokens.
2. A new directory there becomes the config dir and working directory of `claude auth login`, which runs with the user's terminal and browser. `CLAUDE*` variables other than `CLAUDE_CONFIG_DIR`, and `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN`, are removed from its environment. That keeps `CLAUDE_CODE_OAUTH_REFRESH_TOKEN` from spending someone else's token, and `CLAUDE_SECURESTORAGE_CONFIG_DIR` (*undocumented*) from pointing the login back at the main login's store. The directory is never inside `accounts/` or linked to it: deleting through a link would delete the vault (9.7). A non-zero exit saves nothing.
3. ccam reads the login and identity from that directory like any install's (the credentials file; on macOS the Keychain item named for the directory, else the file). If Claude Code kept the login somewhere ccam cannot read, such as Windows Credential Manager, which 2.1.314 can use behind a server-side flag (*undocumented*), ccam stops with a message and saves nothing.
4. The login is a new session, so invariant 2 applies: the owner must be confirmed through the profile endpoint, with no offline save. If the endpoint cannot be reached, nothing is saved (exit 3). The call happens before the vault lock or any of Claude Code's locks is taken, so invariant 5 holds. Then ccam takes the vault lock and Claude Code's refresh locks, in the order of invariant 4, and checks that the account is live nowhere: not on the main login (saved or not), not in any lane, not on another install. The main login is read under Claude Code's locks so it is not read halfway through a `/login`. Under those locks the save is refused when the vault has an entry named `name` that cannot be read (it may hold the only working copy of a login), when `name` holds a different account, when the account is saved under another name (the browser was probably still signed in to it; the message suggests a private window), and when the account is live anywhere. An account that is live on the main login and not saved is refused with `ccam save <name>` as the fix. Otherwise the vault entry is written, and `switch.log` gets `login '<name>' (new)` or `login '<name>' (refreshed)`. Before the browser opens, `ccam login <name>` already refuses a saved `name` that is live somewhere, and an unreadable entry named `name`, so the user does not sign in for nothing.
5. Nothing else changes: no presence record, no write to Claude Code's files, no change to the main login, lanes or open sessions.
6. On every exit path the directory and Claude Code's `<dir>.lock` are deleted, and on macOS the Keychain item as well.

Why a refresh is limited to an account live nowhere. A fresh login is a second family of tokens for the account. If a live install holds the first one, the vault's copy would be a later generation than the live login, which `to` refuses as a conflict (invariant 1). A fresh login does not invalidate the existing family (*inferred*: `claude auth login` calls no revoke endpoint).

Why logout is never used. `claude auth logout` revokes the refresh token on Anthropic's side before it deletes the local copy (*undocumented*, observed in 2.1.314). By then the vault holds the same token, so logout would kill the saved login. For the same reason ccam runs no other `claude` command in that directory after the sign-in: any command can start a token renewal at startup and exit before saving it (anthropics/claude-code#95822). The directory is deleted instead.

Remaining cost: signing in to a different account on the machine can drop Remote Control in main-login sessions (anthropics/claude-code#88855), as a lane's first login can (9.7).

Accepted limit: if writing the vault entry fails (disk full, permissions), the directory is still deleted, so the fresh login is lost and the user signs in again. Nothing saved before is touched, because the vault write is atomic. Keeping the directory would leave live tokens outside the vault, which is the larger risk.

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
