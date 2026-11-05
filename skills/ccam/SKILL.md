---
name: ccam
description: Set up and operate ccam for Claude Code or T3 Code. Use for saved Claude accounts, usage checks, isolated account lanes, account switching, or ccam errors.
---

# ccam

ccam keeps several Claude subscription logins and swaps which one Claude Code uses. The live login belongs to the user's running sessions, including this one: switching changes the account every open Claude Code session uses from its next request.

## Rules

- **Read freely, change on request.** `ccam list`, `ccam usage`, `ccam sessions`, `ccam auto status` and `--help` change nothing; run them whenever they help. A command that changes the live login or the saved accounts (`to`, `save`, `rename`, `remove`, `sync`, `setup`, `uninstall`, `move`, `release`, `lane`) runs only when the user asked for that change in this conversation, with the account they named.
- **`ccam login` is the user's call.** It opens a browser sign-in only the user can complete. Suggest it when an account needs adding; run it only when the user asks, from a normal terminal.
- **`--force` and `ccam auto` are the user's call.** `--force` switches the user's *other* install (Windows or WSL) too. `ccam auto` switches accounts unattended, which Anthropic's Consumer Terms may read as automation; the README explains. Suggest them; run them only when the user asks by name.
- **Secrets stay unread.** Never print or open `.credentials.json`, `login.json` or `~/.claude.json`, and never echo a token or email address. `ccam usage --email` shows emails and `ccam usage --info` shows names, emails and billing details; run them only when the user asks to see them. Use `--info` for account or billing questions, and tell the user it cannot show the renewal date (claude.ai Settings > Billing does). `switch.log` is safe to read.
- Inside Claude Code, run ccam through the shell like any command. It works even when this session is at its usage limit.

## Tasks

**Set up ccam.** Check `ccam --version`, `ccam help`, and `claude --version` first. If ccam is missing, follow the repository README's Install section, then run `ccam setup` only when the user requested setup. Existing accounts and live logins must be preserved. Use `ccam`, not the optional `cs` alias, in commands. Done when the CLI runs and the requested hooks are installed. Do not sign in or switch accounts just to test installation.

**Which account, how much is left.** Run `ccam usage`. Report per account: the 5-hour and weekly percentage and when each resets, and which one is live (`current`). Done when every account is reported or its line explains why it is not (`stale`, rate limited, error).

**Switch.** Run `ccam to <name>`. Then tell the user the switch applies to every open session from its next message, and that a session that does not pick it up resumes with `claude --continue`. Done when ccam printed `Switched from ... to '<name>'`.

**Keep one terminal on one account.** When the user wants a project or a second session to stay on another account (and keep its prompt cache) while other sessions stay where they are, tell them to run `ccam run <name>` in a normal terminal; it opens Claude Code in a lane and cannot be run from inside Claude Code. `ccam sessions` shows the lanes and their sessions. To move a lane whose account is exhausted: `ccam move <lane> <other> --yes`, only when the user asked for that move. To hand an account back: close the lane's sessions, then `ccam release <lane>` (or `ccam release <lane> --delete` to remove its folder too). To remove all empty lane directories: `ccam prune`.

**Check lane safety.** `ccam sessions` distinguishes `no account`, `unsaved login`, and `login unreadable`. Only a logged-out lane with no sessions is empty; allocation and `prune` preserve the other states. For an unsaved login, ask for a name and use `ccam save <name> --lane <id>`. If verification fails, preserve the login and report the error. Never delete a lane or its credentials as a workaround.

**Select a T3 lane.** In the model picker, choose the lane's provider tab, then click a model within it. Clicking only the tab does not apply the selection. Confirm the next run's provider instance, not just the tab being browsed. Moving a thread between lane instances may trigger a bounded history handoff: files and visible chat remain, but not all prior context, attachments, native tool state, or prompt cache transfer. Do not promise exact session continuity.

**Set up another machine.** Install ccam and this skill there; use separate browser sign-ins for the accounts the user requests. There is no ccam export/import command. Do not copy the vault, live credentials, lane junctions, locks, or presence records between machines. Independent machines do not coordinate copied refresh tokens. Done when the requested accounts are saved locally and requested lanes are created, without changing source-machine logins.

**Pick the best account to switch to.** Run `ccam usage`, rank accounts by the higher of their two percentages (lower is better), skip any `in use on ...` in `ccam list`, and recommend one. Switch only if the user then agrees.

**Save a newly logged-in account.** After the user ran `/login` in Claude Code: `ccam save <name>` with the name they chose (add `--lane <id>` if saving a login from a lane from an outside terminal, or run `ccam save <name>` directly inside that lane's session). Done when ccam printed `Saved the logged-in account as '<name>'`.

**Add an account.** Suggest `ccam login <name>` (the user types `cs login <name>` if their shell has the alias). Tell the user what it does: the browser opens, they sign in as the new account, and ccam saves it under `<name>` without touching the main login, lanes or open sessions. Tell them to use a private browser window if their browser is signed in to another saved account. Done when ccam printed `Saved '<name>'. Switch to it with: ccam to <name>`. For an account that is saved but dead, `ccam login <name>` prints `Refreshed the saved login of '<name>'.` instead, and works only while the account is in use nowhere. Never run `claude auth logout` for this: it revokes the login ccam just saved.

**Use lanes in T3 Code.** T3 Code starts Claude itself, so use `ccam lane <account> [--lane <id>]`, not `ccam run`. It checks the account out without launching Claude, reuses its existing lane, and adds a T3 instance labelled `Claude · lane <id> · <account>`. Done when the command reports both the lane's account and its model-picker entry. Select the instance and a model for each thread; threads on different instances use different accounts concurrently. Claude children created without a target inherit their parent's instance. When the account runs out, use `ccam move <lane> <other> --yes` only when requested: conversations stay in that lane, but prompt-cache reuse across accounts is not guaranteed. Changing instances instead uses the history handoff described above. `ccam release` disables the instance; `--delete` and `prune` remove it. If it does not appear, consult troubleshooting. Lane copies of the main `settings.json`, `CLAUDE.md`, and `keybindings.json` refresh at each session start.

**Fix an error.** Match the message below. When it is not listed, read `docs/TROUBLESHOOTING.md` in the ccam repository, which lists every message. Done when the user knows the cause and the exact command or action that fixes it, and you have run the fix if it is a read-only or user-requested command.

When installed without a repository checkout, use the [README](https://github.com/damnRohit/ccam/blob/main/README.md) for installation and the [troubleshooting guide](https://github.com/damnRohit/ccam/blob/main/docs/TROUBLESHOOTING.md) for errors. Do not guess an unsupported command.

## Errors

| Message contains | Cause | Fix |
|---|---|---|
| `stale login` | The saved account's 8-hour access token expired | `ccam usage --refresh` |
| `login expired` / `the token was rejected` | That saved login is dead; something else spent its refresh token | User runs `/login` as that account, then `ccam save <name>` |
| `is in use by Claude on` | The user's other install (Windows/WSL) is live on that account | Switch the other side, or the user approves `ccam to <name> --force` |
| `is in lane` | A lane holds that account | `ccam move <lane> <other>`, or close its sessions and `ccam release <lane>` |
| `which is already saved, not to a new account` | `ccam login` ended up signed in to an account saved under another name (the browser was signed in to it) | User runs `ccam login <name>` again in a private browser window |
| `is in use on` ... `nothing was changed` / `nothing was saved` | `ccam login <name>` for an account that a lane, the other install or the main login uses. `needs no new sign-in` means the saved login still works | Nothing to refresh, or free the account (`ccam to <other>`, `ccam move`, `ccam release`), then run `ccam login <name>` again |
| `is the one the main login uses, and it is not saved` | The account signed in to is Claude Code's current login and has no saved copy | `ccam save <name>` |
| `could not confirm which account you signed in to` | `ccam login` could not reach Anthropic to confirm the owner (exit 3); nothing was saved | Retry `ccam login <name>` online |
| `exists in the vault but cannot be read` | A folder under `~/.claude/ccam/accounts/` fails to load and may hold the only working login; `ccam remove` cannot find it | User copies its `login.json` somewhere safe, fixes or deletes the folder by hand, then runs `ccam login <name>` or `ccam save <name>` again |
| `the sign-in did not finish` | The browser sign-in was closed or failed; nothing was saved | User runs `ccam login <name>` again |
| `Windows Credential Manager` | Claude Code kept the new login where ccam cannot read it; nothing was saved | User runs `/login` as that account, then `ccam save <name>` |
| `could not update T3 Code` | The lane command worked; only T3's `settings.json` was not updated (bad JSON, unwritable, or T3 kept overwriting it) | Fix the file named in the message, then `ccam lane <account> --lane <id>` again |
| `T3 Code not found; skipped` | No `~/.t3/userdata` (or `$T3CODE_HOME/userdata`) folder; the lane was still made | Start T3 once, then `ccam lane <account> --lane <id>` again |
| T3 does not show a lane | The instance is missing, disabled (lane holds no account) or T3 has not reloaded | Check `~/.t3/userdata/settings.json` for the lane's `ccam-lane-` entry (its `displayName` is `Claude · lane <id> · <account>`), run `ccam lane <account> --lane <id>`, restart T3; see `docs/TROUBLESHOOTING.md` |
| `the logged-in account is not saved` | Switching away would lose the current login | `ccam save <name>` first |
| `login could not be read` | The live credentials are incomplete (interrupted `/login`) | User runs `/login` |
| `could not confirm it belongs to` | From `ccam save`, `to` or `sync`: offline, or a `/login` caught halfway | Retry online; else `/login` again |
| `access token expired, send any message` | The live account's token expired | Any message in Claude renews it |
| `rate limited` | Usage endpoint throttling | Wait a minute |
| `held by another process` | A lock is busy (another ccam, or Claude Code renewing) | Retry in a few seconds |
| `Input/output error` on `/mnt/c` (WSL) | WSL lost the Windows drive | From PowerShell: remount, see `docs/WINDOWS-WSL.md` |

The fix that always works: `/login` in Claude Code as the wanted account, then `ccam save <name>`. Only the user can do `/login`; it is interactive.
