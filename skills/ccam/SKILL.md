---
name: ccam
description: Operate ccam, the Claude Code account switcher. Use when the user asks which Claude account is live, how much usage is left, to switch accounts, to save or rename an account, or when a ccam command printed an error.
---

# ccam

ccam keeps several Claude subscription logins and swaps which one Claude Code uses. The live login belongs to the user's running sessions, including this one: switching changes the account every open Claude Code session uses from its next request.

## Rules

- **Read freely, change on request.** `ccam list`, `ccam usage`, `ccam sessions`, `ccam auto status` and `--help` change nothing; run them whenever they help. A command that changes the live login or the saved accounts (`to`, `save`, `rename`, `remove`, `sync`, `setup`, `uninstall`, `move`, `release`, `lane`) runs only when the user asked for that change in this conversation, with the account they named.
- **`ccam login` is the user's call.** It opens a browser sign-in only the user can complete. Suggest it when an account needs adding; run it only when the user asks, from a normal terminal.
- **`--force` and `ccam auto` are the user's call.** `--force` switches the user's *other* install (Windows or WSL) too. `ccam auto` switches accounts unattended, which Anthropic's Consumer Terms may read as automation; the README explains. Suggest them; run them only when the user asks by name.
- **Secrets stay unread.** Never print or open `.credentials.json`, `login.json` or `~/.claude.json`, and never echo a token or email address. `ccam usage --email` shows emails; run it only when the user asks to see them. `switch.log` is safe to read.
- Inside Claude Code, run ccam through the shell like any command. It works even when this session is at its usage limit.

## Tasks

**Which account, how much is left.** Run `ccam usage`. Report per account: the 5-hour and weekly percentage and when each resets, and which one is live (`current`). Done when every account is reported or its line explains why it is not (`stale`, rate limited, error).

**Switch.** Run `ccam to <name>`. Then tell the user the switch applies to every open session from its next message, and that a session that does not pick it up resumes with `claude --continue`. Done when ccam printed `Switched from ... to '<name>'`.

**Keep one terminal on one account.** When the user wants a project or a second session to stay on another account (and keep its prompt cache) while other sessions stay where they are, tell them to run `ccam run <name>` in a normal terminal; it opens Claude Code in a lane and cannot be run from inside Claude Code. `ccam sessions` shows the lanes and their sessions. To move a lane whose account is exhausted: `ccam move <lane> <other> --yes`, only when the user asked for that move. To hand an account back: close the lane's sessions, then `ccam release <lane>` (or `ccam release <lane> --delete` to remove its folder too). To remove all empty lane directories: `ccam prune`.

**Pick the best account to switch to.** Run `ccam usage`, rank accounts by the higher of their two percentages (lower is better), skip any `in use on ...` in `ccam list`, and recommend one. Switch only if the user then agrees.

**Save a newly logged-in account.** After the user ran `/login` in Claude Code: `ccam save <name>` with the name they chose (add `--lane <id>` if saving a login from a lane from an outside terminal, or run `ccam save <name>` directly inside that lane's session). Done when ccam printed `Saved the logged-in account as '<name>'`.

**Add an account.** Suggest `ccam login <name>` (the user types `cs login <name>` if their shell has the alias). Tell the user what it does: the browser opens, they sign in as the new account, and ccam saves it under `<name>` without touching the main login, lanes or open sessions. Tell them to use a private browser window if their browser is signed in to another saved account. Done when ccam printed `Saved '<name>'. Switch to it with: ccam to <name>`. For an account that is saved but dead, `ccam login <name>` prints `Refreshed the saved login of '<name>'.` instead, and works only while the account is in use nowhere. Never run `claude auth logout` for this: it revokes the login ccam just saved.

**Use lanes in T3 Code.** T3 Code starts Claude itself, so `ccam run` does not apply. When the user wants T3 threads on different accounts at once, tell them to run `ccam lane <account> [--lane <id>]` (it checks a saved account out into a lane without opening Claude Code, reusing the lane that already holds the account, and adds a T3 instance labelled `Claude · lane <id> · <account>`; lane ids with `.`, `_` or `@` get a sanitised instance id with a hash). Done when ccam printed `Lane <id> holds '<account>'.` and `T3 Code: lane <id> is in the model picker as ...`. In T3 they pick that instance in the model picker for each thread. Threads on different instances run on different accounts at once. Claude child agents (`delegate_task`, `create_threads` without a target) stay on their parent's instance. When a lane's account runs out, `ccam move <lane> <other> --yes`, only when the user asked: threads keep their conversation. Switching a thread to another instance inside T3 starts a fresh session with about 16k tokens of history, so prefer `move`. `ccam release <lane>` disables the instance; `--delete` and `ccam prune` remove it. If T3 does not show the lane, read `docs/TROUBLESHOOTING.md`. The lane's copies of the main `settings.json`, `CLAUDE.md` and `keybindings.json` refresh at each session start.

**Fix an error.** Match the message below. When it is not listed, read `docs/TROUBLESHOOTING.md` in the ccam repository, which lists every message. Done when the user knows the cause and the exact command or action that fixes it, and you have run the fix if it is a read-only or user-requested command.

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
