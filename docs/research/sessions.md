# Sessions, config dirs and pinning: research

Researched 2026-10-13 against Claude Code 2.1.296 (native install, Windows 11). Extends `internals.md`; facts already recorded there (credential paths, Keychain naming, locks, hook fields, `CLAUDE_PID`) are not repeated. Primary sources: code.claude.com docs (raw `.md` variants), `anthropics/claude-code` CHANGELOG and issues, platform.claude.com prompt-caching docs, tool source at the commits named below. **(local)** means checked on this machine, listing key names only; no token, email or credential content was read. **(inferred)** is my reading, not stated by a source.

Doc URLs used below:
- A = https://code.claude.com/docs/en/authentication
- E = https://code.claude.com/docs/en/env-vars
- SE = https://code.claude.com/docs/en/sessions
- CD = https://code.claude.com/docs/en/claude-directory
- AV = https://code.claude.com/docs/en/agent-view
- SK = https://code.claude.com/docs/en/skills
- PL = https://code.claude.com/docs/en/plugins/loading
- PC = https://code.claude.com/docs/en/prompt-caching
- APC = https://platform.claude.com/docs/en/build-with-claude/prompt-caching
- CSM = https://code.claude.com/docs/en/cross-session-messaging
- CL = https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- I#n = https://github.com/anthropics/claude-code/issues/n

---

## 1. What `CLAUDE_CONFIG_DIR` moves

Documented:
- "All settings, session history, and plugins are stored under this path" (E, `CLAUDE_CONFIG_DIR` row). "If you set `CLAUDE_CONFIG_DIR`, every `~/.claude` path on this page lives under that directory instead" (CD), which covers `projects/`, `file-history/`, `session-env/`, `tasks/`, `plans/`, `paste-cache/`, `backups/`, `sessions/`, `skills/`, `plugins/`, `history.jsonl` and the rest of the CD tables.
- `.credentials.json` moves, and the macOS Keychain entry is keyed to the directory (A#credential-management). Already in `internals.md` 2.1.
- Background sessions: "the supervisor uses that directory instead of `~/.claude` and runs as a separate instance with its own sessions" (AV, "Where state is stored"). The supervisor authenticates "with the same stored credentials as your interactive sessions" (AV), so a pin's `claude --bg` sessions run on the pin's login **(inferred)**.
- Can be set in the shell, user settings or managed settings, never project or local settings (E; CL 2.1.251). `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` removes it from subprocesses (E, scrub table).
- `CLAUDE_CODE_PROJECT_DIR_NAME` (v2.1.234+) works only together with `CLAUDE_CONFIG_DIR` (SE "Name the project directory yourself").
- `claude auth status --json` reports `configDirectory` (CL 2.1.268). **(local)** It also reports an undocumented `projectsDirectory`. With a temp config dir whose `projects` is a junction, both fields print the unresolved path.

Observed **(local)**: a fresh config dir with only `{"hasCompletedOnboarding":true}` gets `projects/`, `sessions/`, `backups/`, `cache/` and `plugins/` on first launch. `claude -p` writes a transcript even when not logged in, which made the experiments in section 2 possible without credentials.

Known bugs where the variable is not honoured everywhere (all open unless noted):
- Session rename writes `custom-title` into `~/.claude` instead of the config dir (IDE extension 2.1.238): I#88603.
- Remote Control drops an unrelated session when a different config dir re-authenticates ("signed-in claude.ai account or organization changed on this machine"): I#88855. **(inferred)** A pin's checkout or renewal can disconnect Remote Control in main-login sessions.
- Sessions in separate config dirs drift onto the wrong account on macOS (2.1.295): I#100762. The cause is not diagnosed.
- Keychain entries were not namespaced per config dir in older builds: I#20553 (open but predates the hashed suffix).
- Earlier ones, closed: `/resume` picker ignored the config dir (I#27978), user `CLAUDE.md` loaded from `~/.claude` (I#47056, I#58815), IDE lock files (CL 2.1.136), `/doctor` (I#79275).
- `CLAUDE_SECURESTORAGE_CONFIG_DIR` is still undocumented: I#79223.

## 2. Linking `projects`, `plugins`, `skills` into a second config dir

### 2.1 Local experiment (Windows, 2.1.296, no credentials)

Setup in `%TEMP%\cct`: config `a` with real `projects/`, config `b` whose whole `projects` is a junction to `a\projects`, config `c` whose `projects` is a real dir holding one per-project junction to `a\projects\<key>`. Sessions were made in `a` with `claude -p` and one interactive prompt driven through a ConPTY. The picker was read by driving `claude --resume` in a ConPTY and stripping escape codes. Symlinks (as opposed to junctions) could not be tested: `New-Item -ItemType SymbolicLink` failed with "Administrator privilege required".

| Operation | `b`: whole `projects` junction | `c`: per-project junction |
|---|---|---|
| `claude -p --continue` appends to the newest session | yes | yes |
| `claude -p --resume <title>` finds the session | yes | yes |
| `claude --resume` picker, current project | lists it | lists it |
| Picker after Ctrl+A (all projects) | lists it, with its directory | "No conversations found." |

So the cresume.py claim ("a linked folder lists nothing", 2.1.281, WSL symlink) does **not** hold for the default picker view with a Windows junction in a non-git folder, but the per-project link does fail the all-projects view. Both outcomes match the loader I#96690 describes: when it enumerates `projects/`, it keeps `readdir(projectsDir, {withFileTypes:true}).filter(d => d.isDirectory())`. That filter drops a linked child (a junction or symlink is not `isDirectory()`) but works when `projects` itself is the link, because its children are real directories. The same enumeration runs for the current project when the repo has two or more git worktrees (I#96690, Windows junction, open), which probably explains the WSL observation **(inferred)**: those repos often have `.claude/worktrees`. Older reports of the same thing: I#59393, I#46342 (closed as duplicates, still reproducible per I#96690).

Also observed **(local)**: `-p` sessions never appear in the picker or plain `--continue`, even in a real directory. This is documented (SE "`/loop`, `-p`, Agent SDK, and background sessions"). A test of the picker must use an interactive session.

### 2.2 What else breaks through links

- **Auto memory writes prompt every time** when the config dir's `projects/` is a symlink to `~/.claude/projects` (I#98044, open, 2.1.280+, macOS). Since 2.1.280 the write check judges both the requested path and where it lands (CL 2.1.280 "writes through a symlinked path ..."). The auto-memory carve-out matches only `<CLAUDE_CONFIG_DIR>/projects/<slug>/memory/`, and the landing `~/.claude/...` path trips the sensitive-file check. Allow rules, `PreToolUse` allow and auto mode cannot approve it. This is exactly the draft pin layout. **(inferred)** A junction is resolved the same way by `realpath`, so Windows pins are likely affected. Not verifiable here without a logged-in session; it needs a manual test.
  - **(local, 2026-10-13, Windows, 2.1.296) Reproduced with a junction.** Headless `claude -p` on Haiku with `--permission-mode acceptEdits`, `CLAUDE_CONFIG_DIR` set to a temp lane, and the access token passed as `CLAUDE_CODE_OAUTH_TOKEN`. The prompt asked Claude to save one memory. Results:
    - Lane with a plain `projects/`: memory written, no denials.
    - Lane with `projects/` junctioned to `~/.claude/projects`: both `Write` and `Read` of `projects/<slug>/memory/` were denied, and nothing was saved.
  - **(local, same setup) Workaround: `--settings '{"autoMemoryDirectory": "<dir>"}'` per launch**, with `projects/` still junctioned and the lane under `~/.claude/ccam/`. This was tried with two targets:
    - `<lane>/memory/<slug>`, the lane's own memory.
    - The real `~/.claude/projects/<slug>/memory`, memory shared with main, with no link in its path.
  - In both targets `Write`, `Read` and the `MEMORY.md` update succeeded. A `Glob` on the memory dir was denied in both. Adding the dir to `permissions.additionalDirectories` did not help. A baseline with a plain config dir and the default memory location allowed the same forced `Glob`. So with a custom `autoMemoryDirectory`, a `Glob` listing of the memory folder prompts (or is denied in `-p`), while reads and writes do not. With memory shared with main and a forced `Glob`:
    - A lane outside `~/.claude` is still denied.
    - A lane under `~/.claude/ccam` with `permissions.allow` `["Read(//C:/Users/.../memory/**)", "Read(//c/Users/.../memory/**)"]` in the same `--settings` has no denials, and memory is written. The documented POSIX form alone (`Read(//c/Users/.../memory/**)`, lower-case drive) is enough.
  - `autoMemoryDirectory` is one fixed path ("must be an absolute path or start with `~/`", any settings scope, MD "Storage location"), so a per-project value has to be passed at each launch. The `<slug>` only has to be deterministic per repository; it need not match Claude Code's own.
- **MCP binary or large results are dropped** ("tool-results path refused ... is a link or not a directory") when `projects/<slug>` is a link (I#97062, open). **(inferred)** A whole-`projects` link is probably not affected, because `<slug>` is then a real directory.
- **Cleanup deleting links**: before 2.1.280, Windows background cleanup could delete "a directory symlink or junction used to relocate `~/.claude/session-env`, `image-cache` or another cleaned-up folder" (CL 2.1.280). `projects/` is a cleaned-up folder (CD "Cleaned up automatically"). Pins need Claude Code 2.1.280 or later.
- **Retention sweep through the link**: a pin's Claude Code sweeps the shared transcripts with the pin's own `cleanupPeriodDays` (CD). Copying `settings.json` keeps the value equal. **(local)** The sweep did not run in a logged-out temp config within 30 s (no `.last-cleanup` written), so this is unverified. **(inferred)** `.last-cleanup` is per config dir, so the shared folder is swept up to once per config dir.
- **`claude purge`** deletes "transcripts and auto memory under `projects/`" and, with `--all`, every project (CD "Clear local data"). From inside a pin it deletes the main history through the junction **(inferred)**.
- **Not shared by the draft**: `file-history/<session>/` (checkpoint snapshots), `tasks/`, `session-env/`, `history.jsonl` (prompt history) and `plans/` live per config dir (CD). **(inferred)** A conversation started on the main login and resumed in a pin keeps its transcript but loses `/rewind` code restore for earlier turns, and up-arrow history differs per pin. ccs links `session-env`, `file-history`, `shell-snapshots` and `todos` as "advanced continuity" items for this reason (ccs `src/management/shared-manager/types.ts` `ADVANCED_CONTINUITY_ITEMS`).
- **Cross-session messaging and `claude agents --json`** read the config dir's own `sessions/` registry **(inferred from AV "separate instance with its own sessions")**, so pinned sessions are invisible to main-login sessions' `/list-agents` and to `claude agents --json` run without the pin's `CLAUDE_CONFIG_DIR`.

### 2.3 Plugins and skills

- Synced skills download into `skills/synced/` and synced plugins into `plugins/synced/`, per claude.ai account (SK "Where synced skills load"; PL "Plugins synced from claude.ai"). **(local)** Both directories hold one `.bucket-<uuid>_<uuid>` per identity (11 to 12 here), so content from two accounts can sit side by side in one shared dir. CL 2.1.296 fixed synced skills "never finishing their install ... when several sessions share a config directory". Sharing `skills`/`plugins` across accounts therefore relies on recent fixes. **(inferred)** It is supported by the bucket layout.
- Symlinked and junctioned skill folders load. **(local)** `~/.claude/skills/vscode-extension-builder` and `web-design-guidelines` are junctions and appear in the skill list. CL 2.0.62 and 2.1.265 cover symlinked skill and plugin dirs.
- Plugin install records hold absolute paths, and CL 2.1.283 fixed "cache-miss" after the config dir moved. A junctioned `plugins` keeps the main paths valid **(inferred)**. CL 2.1.228 fixed cache cleanup deleting a symlinked dev checkout.
- claude-swap deliberately does **not** share `plugins/` ("account- or instance-scoped") and on Windows shares only by copy (claude-swap `src/claude_swap/session.py` `SHARED_ITEMS` comment, 3a4e5c1). ccs junctions `plugins` on Windows (ccs `types.ts` `SHARED_ITEMS`, `symlink-helpers.ts` `linkDirectoryWithFallback`, f45fa92).

### 2.4 Alternatives if linking `projects` is unacceptable

1. **Whole-`projects` junction** (draft): works for `--continue`, `--resume <id|title>` and both picker views (local). Open problems: I#98044 auto-memory prompts and the purge/sweep reach.
2. **Per-project links**: worse. They fail the all-projects picker (local), the picker in repos with worktrees (I#96690) and MCP tool results (I#97062).
3. **Share nothing; resume by id**: the pin keeps its own `projects`, and `ccam run <name> --resume <id>` copies one transcript in (or moves it, so it never forks). `--resume <session-id>` "finds a session stored under either name" only within one config dir (SE), so a copy is required. Copies fork history.
4. **No second config dir at all**: run the pinned terminal with `CLAUDE_CODE_OAUTH_TOKEN` (section 5.3). Same `projects`, same memory paths, no links.

## 3. Prompt caching scope

- "Caches are isolated between organizations. Different organizations never share caches, even if they use identical prompts. Caches are also isolated per workspace within an organization on the Claude API" (APC, "Cache storage and sharing"). Claude Code restates it: "Caches are isolated between organizations, and on some providers, between workspaces within an organization" (PC "Cache scope").
- A subscription's cache lives in Anthropic's infrastructure (PC "Where the cache lives"). Main conversation TTL is one hour on a subscription within plan usage, five minutes on usage credits (PC "Which TTL each request gets").
- **Switching account mid-conversation loses the cache** whenever the two logins are different organizations: the next request rebuilds the full prefix as a cache write. Each individual Pro/Max login is its own organization **(inferred**, also stated in `auto-switch.md` 3). Two seats in the same Team org might share a cache, since the docs scope isolation to org and workspace and not to user **(inferred, unverified)**. vdm states "Prompt caches live per account ... its whole cache is written again there (1.25-2x the input price)" (loekj/claude-acct-switcher README "Sessions & affinity", 866ed4c).
- Within one account, the cache is "effectively scoped to one machine and directory", because the system prompt embeds the auto memory paths (PC "Cache scope"). **(inferred)** A pin's memory path is spelled under the pin's config dir, so a pinned session never shares a prefix with a main-login session even on the same account. With `--system-prompt-snapshot on` (the default, `claude --help`), a resumed conversation reuses its recorded system prompt until compaction, so moving a conversation into a pin keeps its own prefix. It still misses the cache because the organization changed.
- Refreshing the OAuth token no longer breaks the cache (CL 2.1.248, 2.1.259). An in-place swap to the *same* account is cache-safe **(inferred)**.

## 4. Enumerating running sessions

| Source | Records | Config dir? |
|---|---|---|
| `claude agents --json` (documented as "the supported way to read session state from outside Claude Code", AV "List sessions as JSON") | `cwd`, `kind` (`interactive`/`background`), `startedAt`, `sessionId`, `name`; `pid` and `status` (`busy`/`waiting`/`idle`) while alive; `id` and `state` for background sessions; `waitingFor` | No; it lists the config dir it runs under **(inferred)** |
| `<config>/sessions/<pid>.json` (CD: "one small file per running session ... removed when its session exits", stale ones cleared on next launch) | **(local)** `pid`, `sessionId`, `cwd`, `startedAt`, `procStart`, `version`, `peerProtocol`, `peerFeatures`, `kind`, `entrypoint`, `pidDomain` (`win32:<host>`), `messagingSocketPath` (`\\.\pipe\LOCAL\cc-msg-<hex>`), `name`, `nameSource`, `nameSince`, `status`, `updatedAt`, `statusUpdatedAt`; a sibling `<pid>.<hex>.key` per session (the named-pipe auth key, CSM; not read) | Only by location |
| `<config>/daemon/roster.json`, `<config>/jobs/<id>/state.json` | Background sessions (AV); **(local)** roster keys `proto`, `supervisorPid`, `updatedAt`, `workers` | By location |
| Hook stdin | `session_id`, `transcript_path`, `cwd` (`internals.md` 1.1). `transcript_path` is under `<config>/projects/`, so it names the config dir **(inferred)** | Indirectly |
| Env in hooks and tools | `CLAUDE_PID`, `CLAUDE_CODE_SESSION_ID` (`internals.md` 4.2); `CLAUDE_CODE_MESSAGING_SOCKET` / `_TOKEN` (E, v2.1.224/2.1.228). **(local)** also `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_EXECPATH`, `CLAUDE_CODE_SESSION_ATTENDED` (undocumented) | `CLAUDE_CONFIG_DIR` is present unless scrubbed (E) |
| OS process list | pid and command line | Not without reading another process's environment |

**(local)** `claude agents --json` here returned 2 interactive sessions (with `pid`) and 3 background ones (no `pid`, `state: blocked`). Background sessions have no pid of their own to signal.

To list every live session across installs, ccam would run `claude agents --json` once per config dir it manages (main plus each pin, `CLAUDE_CONFIG_DIR` set), or scan each `<config>/sessions/*.json` with the pid-liveness check `internals.md` 4.1 describes. The directory scanned is the session's config dir, and therefore its account.

## 5. Changing the account of one running session

### 5.1 The credentials file is per config dir

A running session reads the credentials of its own config dir. A rewrite there is seen on the next request on Windows/Linux (`internals.md` 5). Other config dirs read a different file (A). So rewriting `<pin>/.credentials.json` moves exactly the sessions started with that `CLAUDE_CONFIG_DIR`, all of them at once, and never a session of another dir **(inferred** from A plus `internals.md` 5). A session cannot change its own `CLAUDE_CONFIG_DIR`, so a session started on the main login can only be moved alone by restarting it: kill its `pid`, then `claude --resume <sessionId>` with the target config dir in its `cwd`. ccam's `--restart` already does this within one dir.

### 5.2 `/login` inside a session

It writes the config dir's shared credentials, so every session of that dir follows. CL 2.1.286 fixed macOS sessions "still showing 'Not logged in' ... after `/login` succeeds in another Claude Code window", which confirms the cross-window effect. No hook fires on it (`internals.md` 1.2). It is not a per-session switch.

### 5.3 `CLAUDE_CODE_OAUTH_TOKEN`

- It ranks above the `/login` credential (A "Authentication precedence", item 5), is "used for the whole session", and is never refreshed: "To replace an expired token, generate a new one and restart" (E).
- `claude setup-token` issues a one-year token per account. It "can only make model requests, so it can't establish Remote Control sessions or fetch claude.ai connectors" (A "Generate a long-lived token"). Sessions using it don't sync claude.ai skills or plugins (SK; PL "Sign-in requirements for terminal sync").
- CL 2.1.225 fixed "a transient 401 replacing a long-lived `CLAUDE_CODE_OAUTH_TOKEN` with a stored login's short-lived token". The two paths have interacted badly before.
- **(inferred)** This is the only per-session mechanism that needs no second config dir and spends no refresh token: the session shares `projects`, memory paths and `file-history` with the main login and can never rotate anything. Costs: a separate `setup-token` flow per account (a different credential from the saved `/login`), restart to change, no Remote Control, connectors or synced skills, and `oauthAccount` in the global config still names the main login, so `/status` and status lines may show the wrong identity (unverified).

### 5.4 `apiKeyHelper`, settings `env`

`apiKeyHelper` is re-run on a TTL and on 401 (`internals.md` 5) but returns an API key or bearer token, not a subscription login. Using it disables Remote Control, connectors and plugin/skill sync (CL 2.1.139; SK). Settings `env` is read at launch and is shared by every session of a config dir. Neither gives a per-session subscription switch.

### 5.5 A proxy

vdm (loekj/claude-acct-switcher) sets `ANTHROPIC_BASE_URL=http://localhost:3334` and routes each request by the `X-Claude-Code-Session-Id` header (or `metadata.user_id`) to an account it chooses, with "session affinity" while the cache is warm (`lib.mjs` `extractSessionId`, `dashboard.mjs` proxy header comment, 866ed4c). It refreshes tokens itself (`lib.mjs` `buildRefreshRequestBody`). This gives true per-session, in-flight switching, but the proxy then spends refresh tokens that Claude Code's own credential store also holds.

### 5.6 Cache effect

Any of the above that changes organization costs one full-context cache write on the next request (section 3). Moving a session is never free. The cheapest point is right after a turn, or after the cache has already expired (`prompt_cache_likely_expired` on SessionStart, `internals.md` 1.1).

## 6. Open-source tools

| Tool (commit) | Mechanism | Per-session | Session listing | Refresh tokens |
|---|---|---|---|---|
| realiti4/claude-swap `cswap run` (3a4e5c1) | `CLAUDE_CONFIG_DIR` profile per account under its backup dir, seeded with a plaintext `.credentials.json` (`session.py` module docstring). Shares `settings.json`, `keybindings.json`, `CLAUDE.md`, `skills`, `commands`, `agents`; not `plugins`. `projects` and `history.jsonl` only with `--share-history`, which is POSIX-only and refused on Windows ("would fork the history"). | Yes, per terminal. Refuses to make a second copy of the account that is the default login (`run`, "same-account fast path"). | `scan_live_sessions(session_dir)` scans that profile's `sessions/` (`session.py`) | The session rotates its own copy. It is captured back into the backup after exit, and `cswap switch` refuses to activate an account whose backup has fallen behind a live session (README "session mode"). Strips `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` and the FD variants before launch (`AUTH_OVERRIDE_ENV_VARS`). |
| kaitranntt/ccs (f45fa92) | One `CLAUDE_CONFIG_DIR` instance per account; "users login directly. No credential copying" (`instance-manager.ts` header). Links `commands`, `skills`, `agents`, `plugins`, `settings.json`; optional shared `projects` (whole dir, junction on Windows, copy fallback) plus `session-env`, `file-history`, `shell-snapshots`, `todos` (`types.ts`, `project-context-sync.ts`, `symlink-helpers.ts`). | Yes, per launch | Not found | None copied, so no double spend by construction |
| ming86/cc-account-switcher (c79a8ed, 2025-07) | Swaps the Keychain item or credentials file, backups in `~/.claude-switch-backup/` (`ccswitch.sh`, README) | No; "restart Claude Code" after each switch | No | Not handled: backups go stale on rotation |
| loekj/claude-acct-switcher, `vdm` (866ed4c) | Local proxy via `ANTHROPIC_BASE_URL`, per-request account choice; macOS Keychain | Yes, per session id, in flight | Its own session table from proxied headers | Refreshes itself |
| dcotelo/cprof (bf0dcc2) | Per-repository profiles; snapshots the Keychain before a profile login (README) | Per repo | No | Not examined |

Other `CLAUDE_CONFIG_DIR` profile launchers found by search but not examined: ya-luotao/dotclaude, korkje/claudes, RayChang/claude-profile, jbarnes7952/claudep, skarl86/ccenvs.

---

## Implications for ccam

Invariant numbers refer to AGENTS.md.

### Contradictions with draft SPEC 9.7

1. **Auto memory through a linked `projects`** (I#98044) makes every memory write in a pinned session ask, and auto mode cannot approve it. 9.7 says nothing about this. It needs a manual test on Windows with a junction (`MANUAL-TESTS.md` 8) and a note in TROUBLESHOOTING, or a different layout.
2. **Minimum Claude Code version**: before 2.1.280, Windows cleanup could delete a junctioned cleaned-up folder (CL 2.1.280). `run` should refuse below 2.1.280, and 9.7's *undocumented* note should cite this entry.
3. **History is not fully shared**: `file-history`, `tasks`, `history.jsonl`, `session-env` and `plans` stay per pin. 9.7's "transcripts, `/resume` and memory are shared" is true; `/rewind` across the boundary and prompt history are not shared.
4. **`claude purge` from a pin** deletes main transcripts through the junction. 9.7's "nothing ever deletes a pin directory" covers ccam only.
5. **`plugins` sharing** differs from claude-swap, which keeps plugins per profile. The per-identity buckets make it look safe, but it depends on 2.1.296's shared-config-dir fix.
6. **Remote Control** in main sessions can drop when a pin's login changes (I#88855). The draft's MANUAL-TESTS 8 does not cover it.
7. The draft's claim that `/resume` lists main conversations in a pin is **confirmed** locally for a whole-dir junction, in both picker views. The cresume.py claim does not carry over to that layout. Do not switch to per-project links.

### (a) Pinning a session to an account

| Option | Refresh token spent twice? | Only working copy lost? | Notes |
|---|---|---|---|
| A. Draft: pin config dir, check the login out of the vault, shared `projects` junction | No, if the checkout is exclusive: the presence record keeps `to`, `usage --refresh`, `start --refresh` and `auto` off the account (inv. 8), and hooks with `--config-dir <pin>` sync each renewal back (inv. 1-3). The vault copy goes stale the moment the pin renews, so nothing may read it as usable while the record exists. | `release` must sync before clearing, as drafted. A pin killed mid-renewal leaves the newest copy only in `<pin>/.credentials.json`; ccam must never delete a pin dir (drafted) and must treat a pin with a live record as the authority. | Cost: I#98044, the version floor, the purge reach |
| B. Pin dir sharing nothing; `run --resume <id>` moves one transcript in | Same as A | Same as A | No link problems; history forks per pin |
| C. `CLAUDE_CODE_OAUTH_TOKEN` from `setup-token`, no pin dir | Never: the env token has no refresh token and Claude Code never refreshes it (E) | The saved `/login` of that account is untouched; the token is a separate credential | No links, same memory and history. No Remote Control, connectors or synced skills; one extra browser flow per account; a one-year expiry ccam must track; the token must be stored in the vault like a login (redacted `Debug`, never printed) |

C sidesteps every link issue and needs no presence record for the pinned account, because two holders of a long-lived token cannot race a rotation. It does not stop `to <name>` from also making the same account live on the main login, which only costs usage.

### (b) Listing live sessions and moving one

- **Listing**: for the main config home and each pin, run `claude agents --json` with that `CLAUDE_CONFIG_DIR` (documented, includes `pid`, `sessionId`, `cwd`, `status`), or scan `<config>/sessions/*.json` with the liveness check from `internals.md` 4.1 when Claude Code is absent. The config dir scanned gives the account, through the presence record of that install. Read-only, so no invariant applies. Treat an unreadable record as "maybe live", as claude-swap does.
- **Moving one session started on the main login to account X**: take X into a pin (option A, all eight invariants as drafted), then reuse the `--restart` sequence: kill the session's `pid` under the main refresh locks so no renewal is in flight (inv. 5: no network while held), release, and run `claude --resume <sessionId>` with `CLAUDE_CONFIG_DIR=<pin>` in the session's `cwd`. Only that session moves. Requires the shared `projects` (A) or a transcript move (B). Costs one cache rebuild and loses earlier `file-history` checkpoints.
- **Moving every session of a pin**: rewrite the pin's credentials under the pin's locks. This is `to` against the pin's paths, so invariants 1-8 hold unchanged. Sessions of that pin follow on their next request; other dirs do not.
- **Not recommended**: a proxy (5.5). ccam would spend refresh tokens alongside Claude Code's own store, breaking the single-owner rule behind invariants 1 and 4, unless every proxied session ran on a token Claude Code cannot refresh.
