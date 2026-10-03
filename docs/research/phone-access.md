# Phone access across account switches

Question: ccam swaps the live Claude Code login between saved Pro accounts. Remote Control ties a session to the claude.ai account that registered it, so the phone (signed in to one account) loses the session after a switch. What survives switching, and what can ccam add?

Researched 2026-09-19 against Claude Code 2.1.274 (local) and docs/CHANGELOG up to 2.1.278. `(inferred)` marks conclusions not stated in a source. Local commands run: `claude --help`, `claude remote-control --help` only. No remote session was started.

Sources, abbreviated below:

- RC: https://code.claude.com/docs/en/remote-control
- ERR: https://code.claude.com/docs/en/errors
- SET: https://code.claude.com/docs/en/settings-reference
- ENV: https://code.claude.com/docs/en/env-vars
- AUTH: https://code.claude.com/docs/en/authentication
- HOOKS: https://code.claude.com/docs/en/hooks
- MOB: https://code.claude.com/docs/en/mobile
- WEB: https://code.claude.com/docs/en/claude-code-on-the-web
- CH: https://code.claude.com/docs/en/channels
- CL: https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md

## 1. How Remote Control works

### Starting it

Three CLI modes (RC#start-a-remote-control-session):

| Mode | Command | Notes |
|---|---|---|
| Server | `claude remote-control` | Persistent server, many concurrent sessions (`--spawn same-dir\|worktree\|session`, `--capacity`, default 32). Shows a URL, spacebar toggles a QR code. `--name`, `--continue`, `--session-id`, `--permission-mode`. |
| Interactive | `claude --remote-control [name]` (alias `--rc`) | Normal terminal session that is also reachable remotely. |
| In-session | `/remote-control [name]` (alias `/rc`) | Carries over the current conversation history. |

Confirmed locally: `claude --help` lists `--remote-control [name]` and `--remote-control-session-name-prefix`; `claude remote-control --help` lists the server flags above and notes "You must be logged in with a Claude account that has a subscription".

Global flags placed before `remote-control` are not carried into server-spawned sessions; most make Claude Code refuse to start (RC, ERR#not-carried-over-to-the-sessions-remote-control-starts).

Auto-start: `remoteControlAtStartup: true` in user `~/.claude/settings.json` (or `/config` > "Enable Remote Control for all sessions") connects every interactive session at start. Project/local settings can turn it off but not on. `--remote-control` turns it on for one session even when the key is `false` (SET#remotecontrolatstartup, RC#enable-remote-control-for-all-sessions, CL 2.1.222). Each interactive process registers one remote session (RC).

Requirements: Pro/Max/Team/Enterprise claude.ai login; API keys, `CLAUDE_CODE_OAUTH_TOKEN`/`setup-token` tokens, custom `ANTHROPIC_BASE_URL`, Bedrock/Vertex/Foundry and `DISABLE_TELEMETRY`/`DO_NOT_TRACK`/`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`/`DISABLE_GROWTHBOOK` all block it; workspace trust must be accepted first (RC#requirements, RC#troubleshooting).

Transport: outbound HTTPS only, registers with the Anthropic API and polls; transcript stored on Anthropic servers while connected (RC#connection-and-security).

### Which account the phone needs

- The phone must be signed in "with the same claude.ai account and organization you use for Claude Code" (MOB, RC#mobile-push-notifications).
- Sessions are listed in the app under **Code**, with a computer icon and green dot when online; or open the URL / scan the QR (RC#connect-from-another-device).
- "A Remote Control session that you started while signed in through `/login` belongs to the claude.ai account and organization that were signed in at the time." (ERR#remote-control-stopped-because-the-signed-in-account-changed)
- Auto-connect "signs in with your own claude.ai account, so a session it starts appears only in your own account's Claude apps" (RC#enable-remote-control-for-all-sessions).

There is no pairing step. The QR is a link to the session URL; visibility is purely "same account" (RC#connect-from-another-device). `(inferred)` A phone signed in to account A cannot open a session registered by account B.

### Credential changes mid-session

- Remote Control runs on short-lived credentials renewed from the saved claude.ai login. If the login is rejected, missing, or signed out, it stops with `Remote Control disconnected — ...` and the local session keeps running (ERR#remote-control-couldnt-refresh-your-login).
- If a different account or organization is signed in on this machine, Remote Control stops "as soon as claude.ai confirms that the account or organization changed", with `signed-in claude.ai account or organization changed on this machine — run /remote-control to start a session for the current account, or /login to switch back, then /remote-control` (ERR#remote-control-stopped-because-the-signed-in-account-changed). Since v2.1.234 this happens "within seconds" (CL 2.1.234; earlier: CL 2.1.176, and before 2.1.234 a delayed HTTP 404).
- Renewal of the same account's token is fine: a stale token is refreshed and retried (RC#remote-credentials-fetch-failed, CL 2.1.224, 2.1.238).

### What a ccam switch does to an active session

In-place `ccam to` (rewrites `.credentials.json` and `oauthAccount`):

- `(inferred)` The docs describe the trigger as `/login` in another terminal, but the check is on the signed-in account, so ccam's file swap should produce the same `account ... changed` disconnect within seconds. The local session continues on the new account (ccam SPEC section 5). Verify manually on each Claude Code release.
- The remote session stays under the old account. `/remote-control` in the same terminal starts a new remote session under the new account, which only that account's phone login can see (ERR, same section).
- Documented recovery that keeps the phone: switch back (ERR, same section).

`ccam to --restart` (kill, switch, `claude --continue`):

- On resume, Claude Code reads the conversation's reconnection record. If "the record names a different account: Claude Code starts a new session without the conversation's earlier messages and without showing a message" (RC#resume-outcomes). So the restarted session auto-reconnects Remote Control, but under the new account and without the earlier history on the device.
- If the record has no account (conversations from before v2.1.227) it shows `Previous session is unavailable — run /remote-control to start a new one` and starts nothing (RC#previous-session-is-unavailable).
- If Remote Control was turned off from the status panel before exit, the record is removed and it does not reconnect (RC#resume-outcomes).
- Server mode (`claude remote-control`) sessions can be revived for about 4 hours with `claude remote-control --continue` / `--session-id` in the same directory (RC#resume-sessions-after-stopping-the-server). `(inferred)` Under a different account they would not be the same sessions; the server-side session belongs to the old account.

## 2. Multiple accounts on one phone

- The Claude app shows only the signed-in account's sessions (section 1). Nothing in the Remote Control or mobile docs describes listing sessions from several accounts.
- The account switcher (click initials, pick the account with the check mark) covers accounts that share one email, for example a personal plan and a Team plan (https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account).
- For different emails there is no switcher on mobile: open feature request "Multi-account switching in Claude Mobile app without shared email", which states "The built-in account switcher only works when both accounts share the same email" and lists workarounds (app for one account, claude.ai in the mobile browser for the other; or sign out and in) (https://github.com/anthropics/claude-code/issues/36151, open, 183 comments). Desktop equivalent: https://github.com/anthropics/claude-code/issues/18435.
- `(inferred)` Workable split: the Claude app signed in to account A and a mobile browser tab at claude.ai/code signed in to account B, because Remote Control sessions open in any browser (RC#connect-from-another-device). Push notifications only go to the app (RC#mobile-push-notifications). ccam accounts are several separate Pro logins, so for N accounts this means N browser profiles or containers, which is clumsy.

## 3. Options

### a) Dedicated phone account in its own `CLAUDE_CONFIG_DIR`

Idea: `CLAUDE_CONFIG_DIR=%USERPROFILE%\.claude-phone`, `/login` once there as account P, run `claude remote-control` or `claude --remote-control` from it. ccam never touches that dir.

- `CLAUDE_CONFIG_DIR` moves settings, history, plugins and `.credentials.json`, and is documented for "running multiple accounts side by side" (ENV `CLAUDE_CONFIG_DIR`, AUTH#credential-management).
- The Remote Control docs say nothing specific about `CLAUDE_CONFIG_DIR`. Open bug: signing in to a different account under one `CLAUDE_CONFIG_DIR` disconnects Remote Control in a session running under another `CLAUDE_CONFIG_DIR`, with the `account ... changed on this machine` message (https://github.com/anthropics/claude-code/issues/88855, open, reported on 2.1.240; no matching fix in CL up to 2.1.278). `(inferred)` Every `ccam to` could therefore knock the phone session offline even though its dir was untouched. This is the deciding risk and must be tested before relying on it.
- History is split: the phone process has its own `projects/`, so it does not see conversations from `~/.claude` (ccam SPEC 9.1, ENV). `CLAUDE_CODE_PROJECT_DIR_NAME` only renames the project dir inside the chosen config dir (ENV).
- Log in separately in the phone dir rather than copying a vault login into it. `(inferred)` Two dirs holding the same refresh token race on rotation, the same hazard ccam's locks exist to prevent.
- Usage: all phone work counts against account P only.

Feasibility: low effort (a shell alias). Reliability: unknown until #88855 is tested against ccam's swap; history split is permanent.

### b) `ccam to --restart` relaunching with Remote Control

- Flag exists: `claude --remote-control [name]` (local `--help`, RC). Setting exists: `remoteControlAtStartup` (SET). Server mode can't be combined with `--continue` of a normal conversation (RC flag table), so the relaunch would be `claude --continue --remote-control "<name>"`. `(inferred)` Combining `--continue` with `--remote-control` is not shown in the docs; test it.
- Even without the flag, `--continue` already reconnects under the new account when the conversation had a reconnection record (RC#resume-outcomes). The flag only helps conversations that never had Remote Control on.
- The phone still has to be signed in to the new account. No re-pairing exists, but the phone must switch accounts, which for different emails means sign out and in (section 2). The device also loses earlier history for that conversation (RC#resume-outcomes).

Feasibility: small ccam change. It does not solve the phone problem; it only makes the new session appear promptly and with a predictable name.

### c) Cloud sessions from the phone

- Cloud sessions run on Anthropic infrastructure from the Code tab in the app, need GitHub access (GitHub App or `/web-setup`), and are research preview for Pro/Max/Team (WEB).
- They belong to the claude.ai account: `--cloud`/`--teleport` need the CLI "signed in to the same claude.ai account" (WEB#move-tasks-between-terminal-and-cloud). `--teleport` gives the terminal its own copy (WEB).
- They clone the GitHub remote, not the local checkout (WEB). No local MCP servers or tools (RC#remote-control-vs-cloud-sessions).
- `(inferred)` Account-switch-proof in a narrow sense: a cloud session keeps running under the phone's account regardless of what ccam does locally. It does not give access to the local machine.

Feasibility: zero ccam work; different workflow; burns the phone account's quota.

### d) Account-agnostic approaches (drive the local machine directly)

These talk to the local `claude` process or terminal, so they follow whatever login ccam made live and survive switches. `(inferred)` for the account-agnostic claim in each case: they spawn the local CLI, which reads the live credentials.

| Approach | Mechanism | Status | Windows fit |
|---|---|---|---|
| Tailscale + Windows OpenSSH + phone SSH client (Termius, Blink) | Private network plus SSH; run `claude --continue` in the project dir | tailscale/tailscale active (pushed 2026-09-19); OpenSSH server is a Windows optional feature (https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_install_firstuse) | No native tmux, so you get a new process, not the existing window. `--continue` picks up the same history in `~/.claude`. Do not continue the same conversation in two live processes at once. |
| VS Code Remote Tunnels | `code tunnel` (or `code tunnel service install`), open vscode.dev in the phone browser, extensions run on the host (https://code.visualstudio.com/docs/remote/tunnels) | Maintained by Microsoft | Works; Claude Code VS Code extension or its terminal on a phone screen is cramped. |
| code-server | VS Code in the browser (https://github.com/coder/code-server, 79k stars, active) | Active | Same as above, self-hosted. |
| ttyd | Share a terminal over the web (https://github.com/tsl0922/ttyd, 12k stars, active) | Active | Put behind Tailscale; no auth by default is a risk. |
| Happy (slopus/happy) | `happy` wraps `claude`; mobile/web app over an end-to-end encrypted relay; "restarts the session in remote mode" when you take over from the phone (https://github.com/slopus/happy) | 23.8k stars, pushed 2026-09-19 | npm CLI; test Windows. Third-party relay server holds encrypted traffic. |
| CloudCLI / Claude Code UI (siteboon/claudecodeui) | Local web server on `:3001` that discovers existing sessions; mobile/web UI (https://github.com/siteboon/claudecodeui) | 13.7k stars, pushed 2026-09-18 | Has a Windows desktop build. Expose via Tailscale. |
| claude-code-telegram (overwirehq) | Telegram bot driving Claude Code via the SDK or CLI, whitelist auth (https://github.com/overwirehq/claude-code-telegram) | 2.8k stars, pushed 2026-09-11 | Needs Python; bot token is another secret. |
| Official Channels (Telegram/Discord) | Channel plugin pushes chat messages into a running session started with `--channels plugin:telegram@claude-plugins-official`; permission relay optional (CH) | Research preview; Pro/Max without an org can use it (CH#enterprise-controls) | Bot identity is independent of the claude.ai account, so `(inferred)` it survives an in-place ccam switch. It is chat-style, not a full session view. |
| VibeTunnel | Browser terminal proxy (https://github.com/amantus-ai/vibetunnel) | 4.7k stars | "Windows is not yet supported" (its README). |
| CUI (wbopan/cui) | Web UI for Claude Code | Archived (https://github.com/wbopan/cui) | Avoid. |

## 4. Hooks and settings around connect/disconnect

- No hook event for Remote Control connect or disconnect. The event list has `SessionStart`, `Notification`, `SessionEnd` etc., none for Remote Control (HOOKS event table). `Notification` matchers are `permission_prompt`, `idle_prompt`, `auth_success`, `elicitation_*`, `agent_needs_input`, `agent_completed`, `quota_auto_resume_*` (HOOKS).
- `CLAUDE_CODE_BRIDGE_SESSION_ID` is set in Bash-tool and hook subprocesses while Remote Control is connected and removed when it ends (v2.1.199+) (ENV, HOOKS). A hook can therefore tell whether the session had Remote Control on when the hook fired.
- Related settings: `remoteControlAtStartup`, `disableRemoteControl`, `agentPushNotifEnabled`, `inputNeededNotifEnabled`, `dialogExpiry`; env `CLAUDE_REMOTE_CONTROL_SESSION_NAME_PREFIX`, `CLAUDE_CLIENT_PRESENCE_FILE` (SET, ENV).
- `SessionEnd` never fires for long-lived remote-control sessions (https://github.com/anthropics/claude-code/issues/73924, cited in `docs/research/internals.md`).

## 5. Recommendation

For someone who switches accounts often, the phone path should not depend on which account is live.

1. Primary: an account-agnostic path to the local machine. Tailscale plus Windows OpenSSH and an SSH app is the lowest-dependency option. For a nicer phone UI, Happy or CloudCLI over Tailscale. Both run the local `claude`, so after `ccam to` they just use the new login, and history stays in one `~/.claude`.
2. If staying with Anthropic Remote Control: accept that the phone follows one account. Keep the phone signed in to one "home" account and treat Remote Control as available only while that account is live. Use a second browser profile at claude.ai/code for a second account if needed.
3. Option (a), a dedicated phone config dir, looks attractive but is blocked in practice by issue #88855 until tested. Add a manual test: run `claude --remote-control` under `CLAUDE_CONFIG_DIR=.claude-phone`, then `ccam to <other>` in the default dir, and watch whether the phone session disconnects.

What ccam could add, smallest first:

- After an in-place `to`, print that any Remote Control session in running Claude Code sessions has stopped and that `/remote-control` restarts it under the new account (only visible on a phone signed in to that account). Cheap, prevents confusion. Risk: none.
- `ccam to --restart --rc`: append `--remote-control "<account>-<dir>"` to the relaunched `claude --continue`, so the new session has a recognisable name on the phone. Risk: flag combination untested; a name containing the account name reveals it in the claude.ai session list (acceptable, it's the user's own account). Do not put emails in the name.
- Optional per-account note in the vault, for example "phone account", so `ccam to` warns "switching away from the phone account: Remote Control will be unreachable from your phone". Risk: extra state to keep in sync.
- Avoid: a `ccam remote` that manages a second config dir or copies vault logins into it. It duplicates Claude Code's own `CLAUDE_CONFIG_DIR` model, risks refresh-token races, and still hits #88855.
- Out of scope for ccam: tunnels, SSH, bridges. Document them in the README instead.
