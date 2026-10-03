# Claude Code: switching between two Claude Pro accounts on Windows

Researched 2026-09-13 against Claude Code docs, the anthropics/claude-code CHANGELOG and issues, Anthropic Consumer Terms, Usage Policy, and Help Center. Local machine: Claude Code `2.1.270`, Windows 11.

## Summary

- There is no built-in multi-account or profile command. Several feature requests were closed as duplicate or not planned ([#24963](https://github.com/anthropics/claude-code/issues/24963), [#35856](https://github.com/anthropics/claude-code/issues/35856), [#44687](https://github.com/anthropics/claude-code/issues/44687), [#33430](https://github.com/anthropics/claude-code/issues/33430)).
- The officially documented approach is **one `CLAUDE_CONFIG_DIR` per account**. The env-vars reference describes it as "Useful for running multiple accounts side by side" ([env-vars](https://code.claude.com/docs/en/env-vars)), and credentials follow that directory ([authentication](https://code.claude.com/docs/en/authentication#credential-management)).
- **Recommended:** keep account A in the default `%USERPROFILE%\.claude` and log account B in once under `%USERPROFILE%\.claude-b`. Add two PowerShell functions, `claude-a` and `claude-b`. Switching means quitting one session and starting the other. There is no logout, login, or file copying.
- **Alternative:** a one-year token per account from `claude setup-token`, passed in `CLAUDE_CODE_OAUTH_TOKEN`. It works but has fewer features: model requests only, with no Remote Control and no claude.ai connectors ([authentication](https://code.claude.com/docs/en/authentication#generate-a-long-lived-token)).
- **Fully automatic switching mid-session is not supported.** Claude Code does expose the signals: the `StopFailure` hook with the `rate_limit` matcher, the status line `rate_limits` field, and `error: rate_limit` in headless output. Hooks can only observe these events, not change credentials. A wrapper can use them to tell you to switch, or to restart with the other profile.
- **Do not copy or swap `.credentials.json` by hand.** Refresh tokens rotate, so a stale copy stops working. Separate config dirs avoid the problem entirely.

## Local machine grounding (read-only, key names only)

- `%USERPROFILE%\.claude\.credentials.json` exists. Top-level keys: `claudeAiOauth`, `mcpOAuth`.
  - `claudeAiOauth` keys: `accessToken`, `refreshToken`, `expiresAt`, `refreshTokenExpiresAt`, `scopes`, `subscriptionType`, `rateLimitTier`.
  - `mcpOAuth` holds per-MCP-server OAuth entries. This means MCP server logins are stored with the account's credentials, not globally.
- `%USERPROFILE%\.claude.json` exists and has an `oauthAccount` object with keys including `accountUuid`, `emailAddress`, `organizationUuid`, `billingType`, `organizationRateLimitTier`, `userRateLimitTier`, `displayName`. The same file also holds `projects`, `mcpServers`, `cachedUsageUtilization`, and UI state.
- The file has duplicate project keys that differ only in drive-letter case (`C:/...` and `c:/...`). Windows PowerShell 5.1 `ConvertFrom-Json` cannot parse it, so any PowerShell script that reads this file needs a different JSON parser, such as `node` or PowerShell 7.
- `CLAUDE_CONFIG_DIR` is not currently set.
- At the time of this check, the logged-in account had been switched via `/logout` + `/login`, so these files reflect the second account. The key structure is identical.

## Findings

### 1. Where credentials and account metadata live

| OS | OAuth credential storage | Source |
|---|---|---|
| Windows | `%USERPROFILE%\.claude\.credentials.json`, protected by the user profile's access controls | [authentication](https://code.claude.com/docs/en/authentication#credential-management) |
| macOS | Encrypted macOS Keychain. Falls back to `~/.claude/.credentials.json` (mode `0600`) when the Keychain rejects the write | same |
| Linux | `~/.claude/.credentials.json`, mode `0600` | same |
| Any, with `CLAUDE_CONFIG_DIR` | `.credentials.json` goes under that directory. On macOS the Keychain entry is keyed to that directory, "so a session with a different `CLAUDE_CONFIG_DIR` reads a different entry" | same |

Account metadata:
- `~/.claude.json` "holds your sign-in session, MCP server configurations, per-project state such as trust decisions, and the global config keys that `/config` writes" ([settings](https://code.claude.com/docs/en/settings)).
- The `.claude` directory reference lists it as "App state, OAuth, UI toggles, personal MCP servers" ([claude-directory](https://code.claude.com/docs/en/claude-directory)).
- The `oauthAccount` key name was confirmed on the local file. The docs do not name that key.

### 2. `CLAUDE_CONFIG_DIR`: support, isolation, caveats

**Officially supported.** The env-vars reference entry reads: "Override the configuration directory (default: `~/.claude`). All settings, session history, and plugins are stored under this path. ... Useful for running multiple accounts side by side: for example, `alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'`. Set it in your shell, user settings, or managed settings. Ignored in project and local settings" ([env-vars](https://code.claude.com/docs/en/env-vars)).

**Credentials are isolated per directory** (see section 1). Two directories give two independent logged-in accounts.

**Where `.claude.json` goes when the variable is set.** The docs do not state this directly. They say "every `~/.claude` path on this page lives under that directory instead" ([claude-directory](https://code.claude.com/docs/en/claude-directory)), but `.claude.json` sits in `~/`, not in `~/.claude/`. The third-party tool claude-swap encodes the rule as `(CLAUDE_CONFIG_DIR || $HOME)/.claude.json` in its [`paths.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/paths.py). That means the second profile gets its own `.claude.json` inside the config dir, including its own `oauthAccount`, MCP servers, and project trust state. Status: inferred from third-party source, consistent with the 1.0.6 changelog entry "Respect CLAUDE_CONFIG_DIR everywhere" ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)).

**Caveats**
- **Nothing is shared.** Settings, plugins, session history, and user-scope MCP servers all live in the per-profile directory ([env-vars](https://code.claude.com/docs/en/env-vars)). A new profile starts without your `CLAUDE.md`, skills, agents, `settings.json`, or plugins, so you have to copy or link them (see the setup section).
- **Sessions do not carry across profiles.** Transcripts are stored under `<config dir>/projects/` ([env-vars, `CLAUDE_CODE_PROJECT_DIR_NAME`](https://code.claude.com/docs/en/env-vars)), so `--continue` or `--resume` in profile B does not see sessions from profile A unless you link the `projects` directory.
- **Project settings cannot set it.** Since 2.1.251, `.claude/settings.json` `env` no longer sets `CLAUDE_CONFIG_DIR` ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)). Set it in the shell instead.
- **Project files are still shared.** `.claude/settings.json`, `.mcp.json`, and `CLAUDE.md` in the repo apply to both profiles, because they belong to the project, not the config dir ([settings](https://code.claude.com/docs/en/settings)).
- **VS Code extension.** Open issue [#55621](https://github.com/anthropics/claude-code/issues/55621) (opened 2026-05-02) says the extension does not respect `CLAUDE_CONFIG_DIR`. Later changelog entries in 2.1.268 and 2.1.269 fix VS Code behavior "when `CLAUDE_CONFIG_DIR` is set in a settings file or the `environmentVariables` setting" ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)). This suggests the extension now honors it through those settings. Not verified hands-on.
- **Earlier fixes for paths that ignored it:** IDE shell-integration lock files (2.1.136) and the workflow save dialog ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)). Other stray paths may exist.
- **Documentation issue history.** [#33430](https://github.com/anthropics/claude-code/issues/33430), a request to document the variable, was closed as not planned. The variable is now documented in env-vars, so the issue is stale.
- **Coverage complaint.** [#44687](https://github.com/anthropics/claude-code/issues/44687) calls it "a partial workaround but requires external scripting and doesn't cover the full config surface" (closed as duplicate, no specifics given).

### 3. Official multi-account feature, `setup-token`, `CLAUDE_CODE_OAUTH_TOKEN`

- **No built-in profiles.** There are no `claude auth switch` or `--profile` commands for claude.ai accounts; requests were closed ([#24963](https://github.com/anthropics/claude-code/issues/24963), [#35856](https://github.com/anthropics/claude-code/issues/35856), [#44687](https://github.com/anthropics/claude-code/issues/44687)).
- **Anthropic profiles are a different feature.** `ANTHROPIC_PROFILE` selects Console or Workload Identity Federation profiles written by the `ant` CLI, not claude.ai Pro logins. Features that need a claude.ai login are unavailable while a profile is selected ([authentication](https://code.claude.com/docs/en/authentication#anthropic-profiles-and-federation-credentials)).
- **`claude setup-token`:**
  - It "opens the same browser authorization flow as `/login`" and generates "a one-year OAuth token".
  - "It does not save the token anywhere". You set it as `CLAUDE_CODE_OAUTH_TOKEN`.
  - It "requires a Pro, Max, Team, or Enterprise plan".
  - "It can only make model requests, so it can't establish Remote Control sessions or fetch claude.ai connectors. MCP servers you configure locally still work."
  - `--bare` mode does not read it.
  - Source for all of the above: [authentication](https://code.claude.com/docs/en/authentication#generate-a-long-lived-token).
- **Precedence.** `CLAUDE_CODE_OAUTH_TOKEN` is position 5, above `/login` credentials at position 7 ([authentication precedence](https://code.claude.com/docs/en/authentication#authentication-precedence)).
  - It "takes precedence over keychain-stored credentials".
  - "Unless you run `/login`, Claude Code uses the token you set for the whole session. To replace an expired token, generate a new one and restart" ([env-vars](https://code.claude.com/docs/en/env-vars)).
- **Switching by env var works.** Run `setup-token` once per account while logged in to that account in the browser, store both tokens, and set `CLAUDE_CODE_OAUTH_TOKEN` to one or the other before launching `claude`. Status: verified-by-docs for the mechanism. Not tested here with two accounts.
- **Relevant fixes:**
  - 2.1.225: "Fixed a transient 401 replacing a long-lived `CLAUDE_CODE_OAUTH_TOKEN` with a stored login's short-lived token" ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)).
  - 2.1.118: `/login` inside a session launched with the env token now clears it so disk credentials take effect ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)).
- **`CLAUDE_CODE_OAUTH_REFRESH_TOKEN` + `CLAUDE_CODE_OAUTH_SCOPES`** let `claude auth login` exchange a refresh token without a browser ([env-vars](https://code.claude.com/docs/en/env-vars)). This is intended for provisioning. Refresh tokens rotate (section 6), so it is not suitable as a stored switch key.

### 4. Usage-limit signals and mid-session switching

| Signal | What it gives | Source |
|---|---|---|
| Status line JSON `rate_limits.five_hour` / `seven_day` | `used_percentage` (0-100) and `resets_at` (Unix seconds). Only for Pro/Max, only after the first API response; each window may be absent. Added in 2.1.80 | [statusline](https://code.claude.com/docs/en/statusline), [CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md) |
| `StopFailure` hook | Fires "when the turn ends due to an API error". The matcher `rate_limit` catches rate-limit failures. Added in 2.1.78 | [hooks](https://code.claude.com/docs/en/hooks), [CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md) |
| Headless `-p` | Exit code 0 on success, non-zero on failure. Stream `system/api_retry` events carry `error: "rate_limit"` | [headless](https://code.claude.com/docs/en/headless) |
| Interactive messages | `You've hit your session limit · resets 3:45pm`, `You've hit your weekly limit`, plus an early warning `You've used 85% of your session limit` | [errors](https://code.claude.com/docs/en/errors#youve-hit-your-session-limit) |
| `/usage` | Plan limits and reset times | [errors](https://code.claude.com/docs/en/errors#youve-hit-your-session-limit) |
| `autoContinueAtUsageLimit` | Waits in the open session and continues after reset (2.1.234+). This is same-account waiting, not switching | [settings-reference](https://code.claude.com/docs/en/settings-reference), [errors](https://code.claude.com/docs/en/errors#youve-hit-your-session-limit) |

**Changing account mid-session:**
- **Supported path: `/login` inside the session.** "If you run `/login` while the variable is set, Claude Code switches the current session to the new login" ([authentication](https://code.claude.com/docs/en/authentication#authentication-precedence)). This still means a browser login, and with a single config dir it overwrites the stored credential.
- **Hooks cannot switch accounts.** They receive error types but the hooks doc describes no mechanism to change authentication ([hooks](https://code.claude.com/docs/en/hooks)).
- **Unknown:** whether a running session picks up a `.credentials.json` that another process swapped underneath it. Not documented.
- **Official Help Center guidance** on hitting a limit: "you'll need to wait for it to reset, upgrade your plan, or purchase usage credits" ([Help Center: usage limits](https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work)).
- **Usage is pooled per account across products.** "your usage of all different Claude product surfaces (claude.ai, Claude Code, Claude Desktop) counts towards the same usage limit" (same source).

### 5. Terms of Service

Relevant clauses, quoted verbatim:

- **Consumer Terms, section 2 (Account):** "You may not share your Account login information, Anthropic API key, or Account credentials with anyone else. You also may not make your Account available to anyone else." ([Consumer Terms](https://www.anthropic.com/legal/consumer-terms))
- **Consumer Terms, section 3, prohibited use:** "Except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it, to access the Services through automated or non-human means, whether through a bot, script, or otherwise." (same)
- **Consumer Terms, section 3:** "You also must not abuse, harm, interfere with, or disrupt our Services, including, for example, ... bypassing any of our systems or protective measures." (same)
- **Usage Policy, "Do Not Abuse our Platform":** "Coordinate malicious activity across multiple accounts to avoid detection or circumvent product guardrails ..." and "Utilize automation in account creation or to engage in spammy behavior" ([Usage Policy](https://www.anthropic.com/legal/aup))

What the text does and does not say:
- None of these clauses mentions one person holding two Pro subscriptions or rotating between them to extend usage. No clause permits it explicitly, and none prohibits it by name.
- The account-sharing clause concerns other people.
- Whether usage limits count as "protective measures", and whether auto-rotation scripts count as "automated or non-human means", is not defined in the text. Anthropic has published no interpretation in the sources checked.
- The Help Center lists waiting, upgrading, or buying usage credits as the options when you hit a limit ([Help Center](https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work)).
- Only Anthropic can give a definitive answer, for example through support.

### 6. Token refresh when copying or swapping credential files

- **The docs describe login refresh without detail.** "Claude Code refreshes the profile's login automatically, and when refresh fails, requests fail ..." and an expired login that "can't be refreshed" fails with `Login expired · Please run /login` ([authentication](https://code.claude.com/docs/en/authentication)). The docs do not say explicitly that refresh tokens are single-use.
- **Issues and changelog point to rotation:**
  - [#24317](https://github.com/anthropics/claude-code/issues/24317) (closed) describes a race in which one process refreshes, the old refresh token is invalidated server-side, and another process holding the old token is forced to re-login.
  - [#54443](https://github.com/anthropics/claude-code/issues/54443) (closed, not planned) logs `POST https://platform.claude.com/v1/oauth/token -> 400` when a process retries with a stale refresh token after another process rotated the file.
  - Changelog 2.1.126: "Fixed a rare race where a concurrent credential write could clear a valid OAuth refresh token" ([CHANGELOG](https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md)).
- **Third-party corroboration from source code.** claude-swap's [`oauth.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/oauth.py) replaces the stored `refreshToken` whenever the refresh response includes a new one. Its [`claude_locks.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/claude_locks.py) holds Claude Code's `<config-home>/.oauth_refresh.lock` while swapping, so that a backup never keeps "a pre-rotation refresh token".
- **Conclusion (inferred, strongly supported):** a copied `.credentials.json` goes stale after the live copy refreshes. Restoring the stale copy later is likely to fail with a 400 and force `/login`. Separate `CLAUDE_CONFIG_DIR` directories avoid this, because each account's single credential file is only ever refreshed in place.

## Recommended setup for Windows PowerShell

### Option A: two config directories (recommended)

**One-time setup** (verified-by-docs mechanism):

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.claude-b" | Out-Null
$env:CLAUDE_CONFIG_DIR = "$env:USERPROFILE\.claude-b"
claude            # log in with account B once, then /exit
Remove-Item Env:CLAUDE_CONFIG_DIR
```

Account A stays in the default `%USERPROFILE%\.claude`. If the default currently holds account B, run `/logout` + `/login` there once with account A, or give A its own `.claude-a` directory instead.

**Functions for `$PROFILE`** (inferred; standard PowerShell env scoping):

```powershell
function Invoke-ClaudeAs {
    param([string]$ConfigDir, [object[]]$Rest)
    $prev = $env:CLAUDE_CONFIG_DIR
    try {
        if ($ConfigDir) { $env:CLAUDE_CONFIG_DIR = $ConfigDir }
        else { Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue }
        & claude @Rest
    }
    finally {
        if ($null -eq $prev) { Remove-Item Env:CLAUDE_CONFIG_DIR -ErrorAction SilentlyContinue }
        else { $env:CLAUDE_CONFIG_DIR = $prev }
    }
}
function claude-a { Invoke-ClaudeAs $null $args }
function claude-b { Invoke-ClaudeAs "$env:USERPROFILE\.claude-b" $args }
```

- **Usage:** `claude-a`, `claude-b`, `claude-b -p "..."`. Check which account is active with `/status`.
- **When A hits its limit:** `/exit`, then `claude-b`.

**Sharing your setup with profile B** (inferred, not tested):
- Directory junctions need no admin rights: `New-Item -ItemType Junction -Path "$env:USERPROFILE\.claude-b\skills" -Target "$env:USERPROFILE\.claude\skills"`. Do the same for `agents` and `commands`.
- Copy `CLAUDE.md` and `settings.json` rather than hardlinking them. Editors that write atomically break hardlinks.
- Never link `.credentials.json`, `.claude.json`, or the `backups` folder.

**Resuming the same conversation on the other account** (inferred, not tested):
- Junction `projects`: `New-Item -ItemType Junction -Path "$env:USERPROFILE\.claude-b\projects" -Target "$env:USERPROFILE\.claude\projects"`.
- After that, `claude-b --continue` should see A's transcripts, because transcripts live under `<config dir>/projects/` ([env-vars](https://code.claude.com/docs/en/env-vars)).
- Avoid running both profiles on the same project at the same time with this link in place.

**Early warning and a switch prompt** (verified-by-docs signals; the script logic is yours to write):
- **Status line:** read `.rate_limits.five_hour.used_percentage` and `.rate_limits.seven_day.used_percentage` from stdin and print a warning above about 90% ([statusline](https://code.claude.com/docs/en/statusline#rate-limit-usage)). Put the account label in the line too, since each profile has its own `settings.json`.
- **Hook:** add a `StopFailure` hook with matcher `rate_limit` in each profile's `settings.json`. Its command should show a toast or print "switch to claude-b --continue" ([hooks](https://code.claude.com/docs/en/hooks)). The hook cannot perform the switch inside the running session.
- **Headless:** a wrapper can check for a non-zero exit, or for `error: "rate_limit"` in `--output-format stream-json`, and rerun the same prompt with the other profile ([headless](https://code.claude.com/docs/en/headless)).

### Option B: long-lived token per account (verified-by-docs mechanism, fewer features)

1. **Generate a token for each account.** Log in to account A in the browser, run `claude setup-token`, and copy the token. Repeat for account B ([authentication](https://code.claude.com/docs/en/authentication#generate-a-long-lived-token)).
2. **Store the tokens encrypted.** Inferred: DPAPI-encrypted with `ConvertTo-SecureString`/`Export-Clixml`, or Windows Credential Manager. Never keep them in plain text in `$PROFILE`.
3. **Launch with the right token.** Set `$env:CLAUDE_CODE_OAUTH_TOKEN` from the stored secret inside the wrapper function, run `claude`, then remove the variable.

Limits:
- Model requests only: no Remote Control, no claude.ai connectors.
- Tokens expire after one year.
- The `rate_limits` status line field and `/usage` were not verified to work with this credential type.

### Third-party tools (not Anthropic, use at own risk)

- **[realiti4/claude-swap](https://github.com/realiti4/claude-swap)** (Python, claims Windows, macOS, and Linux support). Mechanism verified by reading the source:
  - It backs up each account's credential and `oauthAccount` data and writes them into the live `.credentials.json` / `.claude.json` while holding Claude Code's lock files ([`claude_locks.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/claude_locks.py), [`switcher.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/switcher.py)).
  - It refreshes tokens itself against `https://platform.claude.com/v1/oauth/token` ([`oauth.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/oauth.py)).
  - It polls the undocumented `https://api.anthropic.com/api/oauth/usage` endpoint and auto-switches when the 5h or 7d utilization crosses a threshold, 90% by default ([`autoswitch.py`](https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/autoswitch.py)).
  - It depends on undocumented internals (lock file names, refresh endpoint, usage endpoint) that can change between releases.
  - Its automated rotation is the behavior the ToS clauses in section 5 are most likely to be read against.
- **Others found but not source-verified:** [CCSwitcher](https://github.com/XueshiQiao/CCSwitcher) (macOS), [cc-account-switcher](https://github.com/ming86/cc-account-switcher), [brenoluizdev/claude-account-switcher](https://github.com/brenoluizdev/claude-account-switcher) (Windows tray).

## Open questions and unknowns

- **`.claude.json` location** with `CLAUDE_CONFIG_DIR` set is not stated in the official docs; it is inferred from third-party source.
- **Swapped credentials mid-session:** whether a running session picks up a `.credentials.json` changed by another process is undocumented.
- **Token users and usage data:** whether `CLAUDE_CODE_OAUTH_TOKEN` sessions receive `rate_limits` status line data and `/usage` output is unverified.
- **VS Code extension** handling of `CLAUDE_CONFIG_DIR` in current versions: issue #55621 is open, but the 2.1.268/2.1.269 changelog implies partial support. Not tested.
- **Refresh token rotation** as single-use is not stated by Anthropic docs. It is inferred from issues, changelog fixes, and third-party code.
- **Anthropic's position on rotating two personal Pro subscriptions** to extend usage is not addressed explicitly in the Consumer Terms, Usage Policy, or the Help Center articles checked.
- **Junction sharing** of `projects`, `skills`, and similar directories between profiles is untested.
