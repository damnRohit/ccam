---
name: ccam
description: Operate ccam, the Claude Code account switcher. Use when the user asks which Claude account is live, how much usage is left, to switch accounts, to save or rename an account, or when a ccam command printed an error.
---

# ccam

ccam keeps several Claude subscription logins and swaps which one Claude Code uses. The live login belongs to the user's running sessions, including this one: switching changes the account every open Claude Code session uses from its next request.

## Rules

- **Read freely, change on request.** `ccam list`, `ccam usage`, `ccam auto status` and `--help` change nothing; run them whenever they help. A command that changes the live login or the saved accounts (`to`, `save`, `rename`, `remove`, `sync`, `setup`, `uninstall`) runs only when the user asked for that change in this conversation, with the account they named.
- **`--force` and `ccam auto` are the user's call.** `--force` switches the user's *other* install (Windows or WSL) too. `ccam auto` switches accounts unattended, which Anthropic's Consumer Terms may read as automation; the README explains. Suggest them; run them only when the user asks by name.
- **Secrets stay unread.** Never print or open `.credentials.json`, `login.json` or `~/.claude.json`, and never echo a token or email address. `switch.log` is safe to read.
- Inside Claude Code, run ccam through the shell like any command. It works even when this session is at its usage limit.

## Tasks

**Which account, how much is left.** Run `ccam usage`. Report per account: the 5-hour and weekly percentage and when each resets, and which one is live (`current`). Done when every account is reported or its line explains why it is not (`stale`, rate limited, error).

**Switch.** Run `ccam to <name>`. Then tell the user the switch applies to every open session from its next message, and that a session that does not pick it up resumes with `claude --continue`. Done when ccam printed `Switched from ... to '<name>'`.

**Pick the best account to switch to.** Run `ccam usage`, rank accounts by the higher of their two percentages (lower is better), skip any `in use on ...` in `ccam list`, and recommend one. Switch only if the user then agrees.

**Save a newly logged-in account.** After the user ran `/login` in Claude Code: `ccam save <name>` with the name they chose. Done when ccam printed `Saved the logged-in account as '<name>'`.

**Fix an error.** Match the message below. When it is not listed, read `docs/TROUBLESHOOTING.md` in the ccam repository, which lists every message. Done when the user knows the cause and the exact command or action that fixes it, and you have run the fix if it is a read-only or user-requested command.

## Errors

| Message contains | Cause | Fix |
|---|---|---|
| `stale login` | The saved account's 8-hour access token expired | `ccam usage --refresh` |
| `login expired` / `the token was rejected` | That saved login is dead; something else spent its refresh token | User runs `/login` as that account, then `ccam save <name>` |
| `is in use by Claude on` | The user's other install (Windows/WSL) is live on that account | Switch the other side, or the user approves `ccam to <name> --force` |
| `the logged-in account is not saved` | Switching away would lose the current login | `ccam save <name>` first |
| `login could not be read` | The live credentials are incomplete (interrupted `/login`) | User runs `/login` |
| `could not confirm it belongs to` | Offline, or a `/login` caught halfway | Retry online; else `/login` again |
| `access token expired, send any message` | The live account's token expired | Any message in Claude renews it |
| `rate limited` | Usage endpoint throttling | Wait a minute |
| `held by another process` | A lock is busy (another ccam, or Claude Code renewing) | Retry in a few seconds |
| `Input/output error` on `/mnt/c` (WSL) | WSL lost the Windows drive | From PowerShell: remount, see `docs/WINDOWS-WSL.md` |

The fix that always works: `/login` in Claude Code as the wanted account, then `ccam save <name>`. Only the user can do `/login`; it is interactive.
