# Claude Code multi-account switcher: internals research

Researched 2026-09-17 against Claude Code 2.1.274 (local install, Windows 11). Primary sources: code.claude.com docs (raw `.md` variants), `anthropics/claude-code` CHANGELOG and issues, `realiti4/claude-swap` source at HEAD, npm tarballs. Every claim has a URL. Claims marked **(inferred)** are my reading, not stated by a source. **(local)** means checked on this machine by listing key names only; no secret values or emails were read out.

Doc URLs used below:
- H = https://code.claude.com/docs/en/hooks
- A = https://code.claude.com/docs/en/authentication
- E = https://code.claude.com/docs/en/env-vars
- S = https://code.claude.com/docs/en/settings-reference
- ERR = https://code.claude.com/docs/en/errors
- CL = https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- CS = https://raw.githubusercontent.com/realiti4/claude-swap/HEAD/src/claude_swap/

---

## 1. Hooks

### 1.1 Every hook event, matcher, and event-specific stdin fields (H, "Hook events" and matcher table)

Common stdin fields on all events: `session_id`, `prompt_id` (absent until first user input, v2.1.196+), `transcript_path`, `cwd`, `scratchpad_dir` (v2.1.257+), `permission_mode` (not on all events), `effort` (tool-context events), `hook_event_name`; plus `agent_id` / `agent_type` inside subagents or with `--agent` (H#common-input-fields).

| Event | Matcher filters on | Matcher values | Extra stdin fields |
|---|---|---|---|
| SessionStart | how session started | `startup`, `resume`, `clear`, `compact`, `fork` | `source`, optional `model`, `agent_type`, `session_title`; on resume/fork also `seconds_since_last_response`, `context_tokens`, `prompt_cache_likely_expired`, `estimated_cache_write_usd` |
| Setup | CLI flag | `init`, `maintenance` | `trigger` |
| InstructionsLoaded | load reason | `session_start`, `nested_traversal`, `path_glob_match`, `include`, `compact` | file/load fields |
| UserPromptSubmit | none | always fires | `prompt` |
| UserPromptExpansion | command name | skill/command names | `expansion_type` (`slash_command`/`mcp_prompt`), `command_name`, `command_args`, `command_source`, `prompt` |
| MessageDisplay | none | always | turn/message ids, `index`, `final`, `delta` |
| PreToolUse | tool name | `Bash`, `Edit\|Write`, `mcp__.*` | `tool_name`, `tool_input`, `tool_use_id` |
| PermissionRequest | tool name | same | `tool_name`, `tool_input`, optional `permission_suggestions` |
| PermissionDenied | tool name | same | `tool_name`, `tool_input`, `tool_use_id`, `reason` |
| PostToolUse | tool name | same | tool fields plus response |
| PostToolUseFailure | tool name | same | tool fields plus error fields |
| PostToolBatch | none | always | `tool_calls` |
| Notification | notification type | `permission_prompt`, `idle_prompt`, `auth_success`, `elicitation_dialog`, `elicitation_url_dialog`, `elicitation_complete`, `elicitation_response`, `agent_needs_input`, `agent_completed`, `quota_auto_resume_fired`, `quota_auto_resume_stale`, `quota_auto_resume_disabled` | `message`, optional `title`, `notification_type` |
| SubagentStart | agent type | `general-purpose`, `Explore`, `Plan`, custom | `agent_id`, `agent_type` |
| SubagentStop | agent type | same | `stop_hook_active`, `agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message` |
| TaskCreated / TaskCompleted | none | always | `task_id`, `task_subject`, optional `task_description`, `teammate_name`, `team_name` |
| Stop | none | always | `stop_hook_active`, `last_assistant_message`, `background_tasks`, `session_crons` |
| StopFailure | error type | `rate_limit`, `overloaded`, `authentication_failed`, `oauth_org_not_allowed`, `account_on_hold`, `billing_error`, `invalid_request`, `model_not_found`, `server_error`, `max_output_tokens`, `cloud_credential_error` (v2.1.267+), `unknown` | `error`, optional `error_details`, `last_assistant_message` |
| TeammateIdle | none | always | `teammate_name`, `team_name` |
| ConfigChange | config source | `user_settings`, `project_settings`, `local_settings`, `policy_settings`, `skills` | `source`, optional `file_path` |
| CwdChanged | none | always | `old_cwd`, `new_cwd` |
| DirectoryAdded | how added | `slash_command`, `register_repo_root` | `directory`, `source` |
| FileChanged | literal filenames to watch | e.g. `.envrc\|.env` | `file_path`, `event` (`change`/`add`/`unlink`) |
| WorktreeCreate | none | always | `name` |
| WorktreeRemove | none | always | `worktree_path` |
| PreCompact | trigger | `manual`, `auto` | `trigger`, `custom_instructions` |
| PostCompact | trigger | `manual`, `auto` | `trigger`, `compact_summary` |
| PreModelSwitch / PostModelSwitch | canonical model name | `claude-opus-5`, `.*opus.*` | `from_model`, `to_model`, cost fields, `source` |
| Elicitation / ElicitationResult | MCP server name | server names | `mcp_server_name`, `message`/`action`, etc. |
| SessionEnd | exit reason | `clear`, `resume`, `logout`, `prompt_input_exit`, `other` (`bypass_permissions_disabled` removed v2.1.234) | `reason` |

### 1.2 Auth-related hooks: what exists

There is no dedicated "login", "account changed" or "credential refreshed" hook event (H event table lists none).

What comes closest:
- **`Notification` with matcher `auth_success`**: "Authentication completes" (H#notification). Fires even when desktop notifications are off. Output can't block anything. **(inferred)**: this is the `/login` success signal. The doc does not say whether it also fires for MCP OAuth or `claude auth login`, so the handler should re-read identity instead of trusting the event.
- **`SessionEnd` with reason `logout`**: "User logged out" (H#sessionend).
- **`StopFailure` with `authentication_failed`, `oauth_org_not_allowed`, `account_on_hold`, `billing_error`, `rate_limit`**: fires when a turn ends on an API error (H#stopfailure). Useful for "limit hit, suggest switching".
- **`FileChanged` + `watchPaths`**: a filesystem watcher that fires "no matter what changed the file", including a process outside Claude Code (H#filechanged). Matcher names are literal filenames relative to the working directory. Absolute paths anywhere can be added by returning `watchPaths` from `SessionStart`, `CwdChanged` or `FileChanged` (H#sessionstart-decision-control, H#cwdchanged-output). **(inferred)** Returning `watchPaths: ["<config home>/.credentials.json"]` from a SessionStart hook would fire on `/login`, `/logout`, and every token refresh on Windows/Linux (the file store). It would not fire on macOS where the credential lives in the Keychain. Caveats: a `CwdChanged` hook that returns `watchPaths` replaces the dynamic list (H#cwdchanged-output), and every running session watches the file independently, so N sessions fire N hooks for one change.
- **`ConfigChange`** covers settings files and skills only, not `.credentials.json` or `~/.claude.json` (H#configchange).

### 1.3 Slash commands such as /login

- `UserPromptSubmit` receives the submitted `prompt` (H#userpromptsubmit). The docs describe slash commands as taking the `UserPromptExpansion` path: "a per-turn loop containing UserPromptSubmit, UserPromptExpansion for slash commands" (H lifecycle diagram alt text), and "Leave the matcher empty to fire on every prompt-type command" (H#userpromptexpansion).
- **(inferred)** `/login` is a built-in dialog command, not a prompt-type command that expands into a prompt, so `UserPromptExpansion` should not fire for it and there is no documented hook for "user typed /login". Use `Notification:auth_success` for the outcome instead. Verify empirically before relying on it.

### 1.4 SessionStart and SessionEnd details

SessionStart (H#sessionstart):
- `startup` = new session; `resume` = `--resume`, `--continue`, `/resume`; `clear` = `/clear`; `compact` = auto or manual compaction; `fork` = `--fork-session`, `/fork`, `/branch` (before v2.1.214 forks reported `resume`).
- Only `command` and `mcp_tool` handler types are supported.
- In interactive launches, `/clear`, and resume-at-launch, the hooks run in the background, but Claude's first response waits for them.

SessionEnd (H#sessionend):
- Reasons: `clear`, `resume`, `logout`, `prompt_input_exit`, `other`.
- No decision control, and JSON output is discarded.
- Default budget is 1.5 s. A per-hook `timeout` raises it to at most 60 s, or you can set `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS`. Before v2.1.268 that env var did not extend hooks without their own `timeout` (CL 2.1.268).
- Force kill: the docs are silent. **(inferred)** SessionEnd cannot run on SIGKILL or Windows `TerminateProcess`, because no Claude process is left to spawn it. Related issues:
  - Hooks that run too long are cancelled on normal exit without their EXIT traps running: https://github.com/anthropics/claude-code/issues/70465 (closed, not planned).
  - SessionEnd does not fire on `claude -p` exit: https://github.com/anthropics/claude-code/issues/79702 (open).
  - It never fires for long-lived remote-control sessions: https://github.com/anthropics/claude-code/issues/73924.
  - Treat SessionEnd as best-effort only.

### 1.5 Config shape, timeouts, async

Shape (H#configuration, H#hook-handler-fields), in `~/.claude/settings.json` (all projects), `.claude/settings.json`, `.claude/settings.local.json`, managed settings, plugin `hooks/hooks.json`, or skill/agent frontmatter (H#hook-locations):

```json
{
  "hooks": {
    "Notification": [
      { "matcher": "auth_success",
        "hooks": [ { "type": "command", "command": "cswitch hook sync", "timeout": 30, "async": true } ] }
    ],
    "SessionStart": [
      { "matcher": "startup|resume",
        "hooks": [ { "type": "command", "command": "cswitch hook session-start" } ] }
    ]
  }
}
```

- Handler types: `command`, `http`, `mcp_tool`, `prompt`, `agent`.
- Command fields: `command`, `args` (exec form, no shell), `async`, `asyncRewake`, and `shell` (`bash` or `powershell`; defaults to bash, or powershell on Windows when Git Bash is absent). Common fields: `if` (tool events only), `timeout`, `statusMessage`, `once` (skills only) (H#command-hook-fields).
- Timeouts:
  - Defaults are 600 s for `command`, `http` and `mcp_tool`, 30 s for `prompt`, and 60 s for `agent`.
  - `UserPromptSubmit`, `PreModelSwitch` and `PostModelSwitch` default to 30 s. `MessageDisplay` defaults to 10 s.
  - SessionEnd hooks share a 1.5 s budget (H#common-fields).
- All matching hooks run in parallel. The same handler defined in several settings files runs once (H#hook-handler-fields).
- Async: `"async": true` runs a command hook in the background without blocking. `timeout` is not enforced on it. Its output fields arrive on the next turn. In `-p` mode, async hooks still running at teardown are killed, so the docs say to "start a fully detached process" when work must outlive the session (H#run-hooks-in-the-background).
- Hook processes inherit Claude Code's environment, minus `OTEL_*` and minus credentials when `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` is set (H#common-input-fields). From v2.1.251 the scrub also removes `CLAUDE_CONFIG_DIR` (E), so a hook may not know a relocated config dir.
- `disableAllHooks: true` disables non-managed hooks. `--bare` skips hooks entirely (H#disable-or-remove-hooks; `claude --help` locally).

---

## 2. Credential storage

### 2.1 Paths (A#credential-management)
- **Windows**: `%USERPROFILE%\.claude\.credentials.json`, protected by the user profile ACLs.
- **Linux**: `~/.claude/.credentials.json`, mode `0600`.
- **macOS**: encrypted Keychain. If the Keychain rejects the write (for example, locked over SSH), it falls back to `~/.claude/.credentials.json` with mode `0600`.
- **`CLAUDE_CONFIG_DIR`**: the `.credentials.json` file (including the macOS fallback) moves under that directory, and the macOS Keychain entry is "keyed to that directory too", so each config dir reads a different entry (A).
- `CLAUDE_CONFIG_DIR` is officially documented as the way to run multiple accounts side by side, e.g. `alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'`. It is ignored in project and local settings (E).
- **(local)** On this machine the file's top-level keys are `claudeAiOauth` and `mcpOAuth`. `claudeAiOauth` has `accessToken`, `refreshToken`, `expiresAt`, `refreshTokenExpiresAt`, `scopes`, `subscriptionType`, `rateLimitTier`.
- The `mcpOAuth` sibling is machine-shared MCP OAuth state. claude-swap treats it as live-authoritative and never swaps it per account (CS credentials.py, `SHARED_CREDENTIAL_KEYS`).

### 2.2 macOS Keychain naming (from claude-swap, mirroring the Claude Code bundle)
- Service `Claude Code-credentials` holds the OAuth JSON for the default config dir (CS credentials.py `CLAUDE_CODE_KEYCHAIN_SERVICE`). Confirmed independently in issue https://github.com/anthropics/claude-code/issues/80585.
- With `CLAUDE_CONFIG_DIR` set, the service is `Claude Code-credentials-<first 8 hex chars of sha256(NFC(raw env string))>`. The raw, unresolved string is hashed, so a trailing slash changes the name (CS session.py `keychain_service_name`).
- `CLAUDE_SECURESTORAGE_CONFIG_DIR`, when defined, overrides which dir secure storage keys on. Defined but empty means the default unsuffixed item (CS credentials.py `_active_oauth_keychain_services`, citing Claude Code 2.1.220). This variable is not in the env-vars doc (E, grep: no match).
- Service `Claude Code` (no suffix) holds a managed API key from a Console `/login`. Off macOS, that key lives in `~/.claude.json` as `primaryApiKey` (CS credentials.py).
- Account name is `$USER`, else the OS username (`pwd`), else `claude-code-user`. This mirrors Claude Code's `getUsername()` (CS macos_keychain.py `keychain_account_name`).
- `security` usage (CS macos_keychain.py):
  - Always runs the absolute path `/usr/bin/security`, with a 5 s timeout.
  - Read: `security find-generic-password -a <account> -w -s <service>`. Exit code 44 means not found. Any other non-zero exit is locked, denied or unavailable, and is not the same as "logged out".
  - Existence check without decrypting: the same command without `-w`.
  - Write: `add-generic-password -U -a <account> -s <service> -X <hex(value)>`, sent through `security -i` on stdin so the secret stays out of argv. It falls back to argv only when the line exceeds about 4032 bytes (the stdin buffer is 4096; see Claude Code issue #30337 cited there).
  - Delete: `delete-generic-password -a -s`. Exit code 44 counts as success.
- Claude Code's own Keychain read cache on macOS was extended from 5 s to 30 s (CL 2.1.86: "macOS keychain cache extended from 5s to 30s").

### 2.3 `~/.claude.json` location
- Default is `$HOME/.claude.json`, which sits in the home directory, not inside `.claude/`.
- With `CLAUDE_CONFIG_DIR` set, it is `$CLAUDE_CONFIG_DIR/.claude.json`.
- A legacy `<config home>/.config.json` wins if it exists (CS paths.py `get_global_config_path`, citing Claude Code `utils/env.ts getGlobalClaudeFile`).

### 2.4 Account identity (`oauthAccount` in `.claude.json`)
- claude-swap identifies accounts by `oauthAccount.emailAddress`, `oauthAccount.organizationUuid` and `oauthAccount.accountUuid`. It matches on the uuid first, and on (email, org) as the slot key, because one email can belong to several orgs (CS switcher.py `_get_current_identity_triple`, `_live_identity_matches`).
- **(local)** Other keys present: `displayName`, `organizationName`, `organizationRole`, `organizationType`, `billingType`, `seatTier`, `workspaceRole`, `userRateLimitTier`, `organizationRateLimitTier`, `profileFetchedAt`, and more.
- **(inferred)** `accountUuid` + `organizationUuid` is the stable key and email is display only. The identity lives in `.claude.json` while the tokens live in `.credentials.json` or the Keychain, so a switch has to write both consistently.
- `claude auth status` has `--json` (default) and `--text` (local `claude auth status --help`). `claude auth login` has `--claudeai`, `--console`, `--email`, `--sso` (local help).

---

## 3. Token refresh and rotation

### 3.1 Evidence that refresh tokens are single-use and rotate
- Issue #80585: "Anthropic's refresh grant appears to be **single-use** (it rotates the refresh token), so the first refresher succeeds and the others fail with an `invalid_grant`-class error" (Claude Code 2.1.218, closed not planned): https://github.com/anthropics/claude-code/issues/80585
- Issue #88583: the Keychain credential is wiped when concurrent sessions "race the single-use refresh token": https://github.com/anthropics/claude-code/issues/88583
- Issue #94464: Desktop's embedded engine rotates the refresh token and the Keychain stays stale, so the CLI shows "Login expired" daily: https://github.com/anthropics/claude-code/issues/94464
- CHANGELOG entries:
  - 2.1.81: "multiple concurrent Claude Code sessions requiring repeated re-authentication when one session refreshes its OAuth token"
  - 2.1.126: "concurrent credential write could clear a valid OAuth refresh token"
  - 2.1.133: "parallel sessions all dead-ending at 401 after a refresh-token race wiped shared credentials"
  - 2.1.136: "concurrent credential write could overwrite a freshly-rotated OAuth token"
  - 2.1.211: "parallel Claude Code sessions all logging out simultaneously after wake-from-sleep when many sessions share one credential store"
  - 2.1.214: "feature flags going stale ... after the OAuth token rotates" (CL)
- ERR#login-expired: when the OAuth service rejects the stored refresh token, "Claude Code cleared the saved credentials".
- claude-swap's own model: "POSTing a superseded one-time rt yields invalid_grant" (CS credentials.py `ActiveCredentials`). Also, "every refresh, and every login, issues a token that expires later than the last", so it orders generations by `expiresAt` (CS switcher.py).
- **Consequence**: a backup snapshot of an account goes dead as soon as a live session refreshes that account. Any stored copy has to be re-synced after each refresh, and a stale refresh token must never be restored or used.

### 3.2 Claude Code's locks (CS claude_locks.py, verified by its author against the 2.1.218 bundle)
- These are `proper-lockfile` npm locks. The lock artifact is a **directory**, and atomic `mkdir` is the mutex. Holders refresh the directory mtime every 5 s.
- OAuth refresh takes two locks, in order:
  1. `<config home>/.oauth_refresh.lock` (primary, 2.1.218+)
  2. `<config home>.lock`, i.e. `~/.claude.lock` as a sibling of the dir (legacy, kept "for compatibility with external tools")
- Both locks use `stale: 60000, update: 5000`. If the legacy lock is contended, Claude Code releases the primary and retries. It retries a held lock 5 times with 1-2 s jitter.
- The config write lock is `<dir of .claude.json>/.claude.json.lock` (default `~/.claude.json.lock`). It is stale after 10 s and touched every 5 s.
- claude-swap's acquisition:
  - Loop on `os.mkdir(lock_dir)`. On `FileExistsError`, stat the mtime and treat the lock as stale (rmdir, then retry) only when older than 60 s for the credential locks or 10 s for the config lock. Otherwise sleep 0.25-0.5 s.
  - Give up after 9 s per lock.
  - While holding, a daemon thread `utime`s the directory every 3 s. On exit it `rmdir`s.
  - It takes both credential locks in Claude Code's order to avoid deadlock.
- Why this works: Claude Code's refresh reads, refreshes and saves under both locks, and re-reads under the lock. A swap done under the lock makes Claude Code see a non-expired swapped credential and abort its refresh (CS claude_locks.py module docstring).
- **(local)** None of the three lock dirs existed at check time, which is expected because they only exist while held.
- Other Claude Code bugs still exist in this area, e.g. #88583 (Desktop) and #85236 (MCP per-process lock). The locks lower the risk but don't remove it.

---

## 4. Process detection and env vars

### 4.1 claude-swap (CS process_detection.py)
- **All OSes**: scans `<config home>/sessions/*.json`, the session registry Claude Code writes, and `<config home>/ide/*.lock` (IDE instances).
- Session record fields it uses: `pid`, `sessionId`, `cwd`, `startedAt`, `kind` (`interactive`, `bg`, `daemon`, `daemon-worker`), `entrypoint` (`cli`, `claude-vscode`, `claude-desktop`, `sdk-cli`, `mcp`), `status`, `procStart`.
- **(local)** This machine's record also has `messagingSocketPath`, `name`, `peerProtocol`, `pidDomain`, `updatedAt`, `version`.
- Liveness check:
  - POSIX: `os.kill(pid, 0)`, where EPERM counts as alive.
  - Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` through ctypes.
- PID reuse guard:
  - Linux: `procStart` must equal `/proc/<pid>/stat` field 22.
  - macOS: compares `ps -o lstart=` under `LC_ALL=C TZ=UTC`, allowing 120 s of slack, then checks that `ps -o comm=,args=` contains "claude".
  - Windows: `procStart` is a FILETIME. claude-swap does not verify it and treats "can't tell" as alive.
- Guards (destructive operations) use `scan_sessions`, which also returns an unreadable-record count. An unreadable record counts as "maybe live", never as "nobody there".
- It does not enumerate OS processes by name.

### 4.2 Documented env vars in child processes (E)
- `CLAUDE_PID`: "Claude Code sets this to its own process ID in the subprocesses it spawns: Bash and PowerShell tool commands and hook commands ... Read it from your own scripts to identify or signal the parent Claude Code process". Requires v2.1.214+. **Documented.**
- `CLAUDECODE=1`: set in Bash/PowerShell tools, tmux, hooks, status line, and stdio MCP servers. IDE extensions also set it in their integrated terminals.
- `CLAUDE_CODE_CHILD_SESSION=1`: set only by Claude Code for Bash, PowerShell and Monitor tools, hooks, and status line (not MCP), so it reliably detects nesting.
- `CLAUDE_CODE_SESSION_ID`: matches the hook `session_id` and updates on `/clear`.
- `CLAUDE_EFFORT` (H#common-input-fields), `CLAUDE_PROJECT_DIR` (used in H examples), `CLAUDE_ENV_FILE` (E), `CLAUDE_CODE_REMOTE`, `CLAUDE_CODE_BRIDGE_SESSION_ID` (H#hook-handler-fields).
- `CLAUDE_CODE_ENTRYPOINT`: **not documented** on the env-vars page (E, grep: 0 matches). The `entrypoint` concept does appear in session records (CS process_detection.py) and in issue #94464.

---

## 5. Reloading credentials in a running session

No documented "reload credentials" command, signal, or hook output exists. Documented and near-documented behaviour:
- `/login` inside a session switches that session to the new login, including over `CLAUDE_CODE_OAUTH_TOKEN` (A#authentication-precedence; CL 2.1.118).
- `CLAUDE_CODE_OAUTH_TOKEN` is used for the whole session. "To replace an expired token, generate a new one and restart" (E).
- `apiKeyHelper` is the only official hot-rotation mechanism. It re-runs after a TTL (5 min default, `CLAUDE_CODE_API_KEY_HELPER_TTL_MS`), on a 401/403, and before sending an expired JWT (S#apikeyhelper). It supplies an API key or bearer token, not a subscription OAuth login. **(inferred)** It doesn't fit subscription account switching.
- The OAuth token is refreshed reactively on 401 (CL 2.1.117: "the token is now refreshed reactively on 401").
- CL 2.1.178 fixed "model requests continuing to fail with auth errors after credentials were refreshed outside the session, due to a stale cached request configuration", which implies external credential changes are picked up. CL 2.1.271 fixed "a cached organization policy being reused after switching accounts ... when the credential changes mid-session".
- claude-swap states: "A restart is never required: Claude Code clears its cached OAuth token when `.credentials.json` changes (file storage, effective on the next message) or when the macOS Keychain cache TTL (~30s) expires" (CS switcher.py `_print_switch_followup`). This is third-party behaviour taken from the bundle, **not official**. The ~30 s figure matches the changelog note on the Keychain cache.
- **(inferred)** An in-place swap works in practice on Windows and Linux on the next request. It is still undocumented and can break between versions, so a `/status` check or restart remains the safe fallback.

---

## 6. Existing npm tools

- **claude-accounts** 1.0.2 (last modified 2025-03-05): an Ink TUI that stored accounts under an `__ext__accounts` key inside `~/.claude.json` and swapped the identity/API key there. Its README now says **DEPRECATED**: "Claude Code API keys are now stored in macOS Keychain ... does not work anymore". Lesson: writing into Claude Code's own config file is fragile. https://www.npmjs.com/package/claude-accounts
- **ccacct** 0.1.0 (2025-12-13): macOS only. `save`/`use`/`switch` copy the `Claude Code-credentials` Keychain item to and from per-profile Keychain items, and `switch` then runs `claude --continue`. The profile index `~/.config/ccacct/profiles.json` holds names only. It has no refresh-token re-sync, no locks, and no Windows/Linux support. https://www.npmjs.com/package/ccacct
- **claude-account-switch** 1.2.3 (2026-07-31): one `CLAUDE_CONFIG_DIR` per profile, with shell integration for sh, fish and PowerShell (`cpf <name>`). It symlinks shared settings, runs per-profile MCP management, and migrates `~/.claude`. Each profile logs in once, so there is no token copying and no rotation race. The trade-off is separate history and projects unless linked, and switching happens per shell, not in place. https://github.com/ftery0/claude-account-switch
- **Reference, not npm:** realiti4/claude-swap (`cswap`, Python) is the most thorough in-place swapper. It has encrypted per-slot backups, Claude Code lock cooperation, rotation-generation tracking, session-registry checks, and a "session profiles" mode that uses hashed Keychain names. https://github.com/realiti4/claude-swap
- An official multi-account feature request is still open: https://github.com/anthropics/claude-code/issues/30031

---

## Design implications

1. **Two viable architectures.**
   - (a) **Per-account `CLAUDE_CONFIG_DIR`** is officially endorsed (E) and has no token copying and no rotation races. Its cost: each account has its own history, settings and plugins unless linked, and you can't switch a running session.
   - (b) **In-place swap of the active store** keeps one history. Its cost: it must handle rotation, locks, Keychain naming, and live sessions.
   - A hybrid is possible: use (a) for isolation and offer (b) as `switch` for the default dir.
2. **Sync trigger without running on every reply.** Don't use `Stop` or `UserPromptSubmit`. Register instead:
   - `SessionStart` (matcher `startup|resume`): a fast, idempotent check that captures or refreshes the current account's snapshot. It also returns `watchPaths: [<abs config home>/.credentials.json]`.
   - `FileChanged` with no matcher (or one scoped to the dynamic path): re-captures after `/login`, `/logout` and each token rotation on Windows/Linux. Use `async: true`. De-duplicate by content fingerprint plus identity, because N sessions fire at once and your own writes trigger it too.
   - `Notification` matcher `auth_success`: capture a new login, and this also covers macOS where no file changes.
   - `SessionEnd` matcher `logout`: mark the slot logged out. Best-effort only.
   - Optionally `StopFailure` matcher `rate_limit|authentication_failed|billing_error`: suggest or auto-switch.
3. **Rotation is fatal to stale copies.** Never restore or use a refresh token older than the live one for the same `accountUuid`. Before switching away, snapshot the live credential under Claude Code's locks. Order generations by `expiresAt`. Treat a failed Keychain read as "unknown", never as "empty".
4. **Locks.** Before touching credentials, acquire `<home>/.oauth_refresh.lock` then `<home>.lock` (mkdir dirs, 60 s staleness, touch every 3-5 s). Take `<.claude.json dir>/.claude.json.lock` (10 s staleness) for `.claude.json`. Write atomically (temp file plus rename) and preserve unknown keys and `mcpOAuth`.
5. **Identity.** Key accounts on `oauthAccount.accountUuid` + `organizationUuid`. Store email only as a label, and never log it or tokens. Swap `oauthAccount` in `.claude.json` together with `claudeAiOauth`.
6. **Platform matrix.**
   - Windows/Linux: `<CLAUDE_CONFIG_DIR or ~/.claude>/.credentials.json`.
   - macOS: Keychain `Claude Code-credentials[-sha8]`, account `$USER`, via `/usr/bin/security` with hex `-X` over `security -i`, plus file fallback detection.
   - `.claude.json`: `$CLAUDE_CONFIG_DIR/.claude.json` or `~/.claude.json`.
7. **Live-session safety.** Detect sessions from `<home>/sessions/*.json` with a PID liveness check. Inside a hook, `CLAUDE_PID` and `CLAUDE_CODE_SESSION_ID` identify the caller. Warn that macOS may take ~30 s to pick up a swap, while Windows/Linux pick it up on the next request (third-party observation, not official).
8. **Hook robustness.** Hooks may run under Git Bash or PowerShell on Windows. Prefer exec form (`command` + `args`) pointing at the CLI binary. Keep SessionStart fast. Remember `--bare` and `disableAllHooks` skip hooks, and `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` may hide `CLAUDE_CONFIG_DIR`, so the CLI must also self-heal by re-checking identity on every command it runs.
