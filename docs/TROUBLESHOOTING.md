# Troubleshooting

Every message ccam prints when something is wrong, what it means, and how to fix it. Messages are quoted as ccam prints them; `<name>` stands for an account name.

**The universal fix.** Whatever state you are in, this works: run `/login` in Claude Code as the account you want, then `ccam save <name>`. It replaces the saved copy with a fresh login. Nothing else is lost.

**First look.** `ccam list` shows the saved accounts and which one is live. `~/.claude/ccam/accounts/switch.log` records every switch, sync and renewal with a timestamp. It holds no secrets and is safe to share.

## Contents

- [Logins and saved accounts](#logins-and-saved-accounts)
- [Switching](#switching)
- [Usage and start](#usage-and-start)
- [Windows and WSL](#windows-and-wsl)
- [Locks and files](#locks-and-files)
- [Automatic switching](#automatic-switching)
- [Hooks](#hooks)
- [Reporting a bug](#reporting-a-bug)

## Logins and saved accounts

### `not logged in to Claude Code`
Claude Code has no login at all. Run `claude`, log in, then `ccam save <name>` or `ccam to <name>`.

### `Claude Code's login could not be read (missing tokens or expiry), so ccam will not replace it. Run /login in Claude Code first`
The live credentials file exists but is incomplete, so ccam cannot tell whether it holds a login worth keeping, and refuses to overwrite it. Usually a `/login` was interrupted. Run `/login` in Claude Code.

A login Claude Code has emptied itself (both tokens blank, after a refused renewal) counts as logged out, not unreadable, so `ccam to <name>` works from that state.

### `Claude Code's current login could not be read. Run /login in Claude Code.` (from `ccam list`)
Same cause as above, shown by `list`.

### `the logged-in account is not saved, so switching would lose its login. Run: ccam save <name>`
You are logged in to an account ccam does not know. Switching away would throw that login away. Save it first, then switch.

### `The logged-in account is not saved. Run: ccam save <name>`
The same, from `ccam list` or `ccam sync`. Harmless until you want to switch away.

### `the logged-in account is already saved as '<name>'`
`ccam save` with a new name for an account that is already saved. Use the existing name, or `ccam rename`.

### `'<name>' holds a different account; remove it first or pick another name`
The name is taken by another account. Pick another name, or `ccam remove <name>` first.

### `an account named '<name>' already exists`
From `ccam rename`. Pick another new name.

### `no saved account named '<name>'`
Check `ccam list`. Names are compared without case.

### `invalid account name ...`
Names start with a letter or digit, then letters, digits, `.`, `_`, `@` or `-`, up to 64 characters.

### `the logged-in login is new and ccam could not confirm it belongs to '<name>' (...). Nothing was changed.`
After a `/login`, ccam asks Anthropic which account the new token belongs to before saving it. That check failed: you are offline, or the token belongs to a different account than `~/.claude.json` says (a `/login` caught halfway). Retry when online. If it keeps failing, run `/login` again.

### `A new login appeared but was not confirmed to belong to '<name>' (...); left unchanged.`
The same check from `ccam sync` or a hook. Nothing was saved. It resolves itself on the next online sync.

### `the saved login for '<name>' is newer than the logged-in one, which should not happen.`
The saved copy is a later generation than the live login. Overwriting it would install a dead token, so ccam stops. Usually two tools (or two machines) used the same account. Run `/login` as that account, then `ccam save <name>`.

### `'<name>' already holds a newer login; left unchanged.`
From `ccam sync`: the same situation, reported without failing. Nothing to do unless switching fails.

## Switching

### `already on '<name>'`
Nothing to do.

### A running Claude Code session did not pick up the switch
Sessions read the new login on their next request (on macOS within about 30 seconds). If one does not, restart it with `claude --continue`; the conversation continues.

### `'<name>' is in use by Claude on <side>; using it here too would log one of them out.`
Windows and WSL share your saved accounts, and the other side is logged in as `<name>`. Both using one saved login would let the first renewal log the other out. Either:
- switch the other side to something else (from Windows, `csw to <other>` or in WSL `ccam to <other>`), or
- run `ccam to <name> --force`, which moves the other side to the free account with the most room first.

### `--force` fails: `cannot run ccam on <side> ...` or `switching <side> to '<x>' failed`
ccam could not reach the other side's ccam (WSL not installed any more, ccam uninstalled there, or the distro renamed). The message names a record file under `~/.claude/ccam/accounts/.live/`. If that install no longer exists, delete the file and retry. If it does exist, switch it by hand there.

### `no free account to move <side> to; switch it by hand`
Every other account is in use or this side is on it. Switch the other side yourself.

### `--open runs from a normal terminal, not inside Claude Code`
`--open` and `--continue` start a new Claude Code, which makes no sense inside one. Inside Claude Code, use plain `ccam to <name>` (the session switches in place) or `--restart` on Windows Terminal.

### `--restart` opens nothing, or the old tab does not close
`--restart` needs Windows Terminal. In other terminals ccam switches in place and prints `claude --continue`. If the new tab opened but the old one stayed, close it by hand; the switch itself happened.

## Usage and start

### `stale login, run: ccam usage --refresh`
That account's access token (valid about 8 hours) expired while you were not using it. `ccam usage --refresh` renews it with its refresh token and saves the result.

### `access token expired, send any message in Claude to renew it`
The live account's token expired. ccam never renews the live login itself (Claude Code owns it); sending any message in Claude makes Claude Code renew it.

### `login expired, run /login in Claude Code and save the account again`
The saved refresh token was refused: something else already spent it (the account was used on another computer, by another tool, or from a stale copy). Run `/login` as that account, then `ccam save <name>`.

### `the token was rejected`
Anthropic refused the access token. Same fix as above.

### `'<name>' is logged in right now (here or elsewhere with the same login), so ccam will not renew its token`
`usage --refresh` and `start --refresh` never renew a login that is live here or on your other install, because renewing it would log that session out. Nothing is wrong; its usage shows from the live side.

### `rate limited, try again in a minute`
The usage endpoint limits how often you can ask. Wait a minute.

### `timed out` / `network error: ...` / `unexpected response (...)`
Network trouble, or Anthropic changed the endpoint. Retry. If it persists across days, the endpoint may have changed; check for a ccam update.

### `ccam start` says `skipped: saved login expired; run with --refresh to renew it`
Run `ccam start --refresh`.

### `ccam start` says `failed: could not read usage: ...`
See the matching usage message above.

## Windows and WSL

These only apply when WSL's `~/.claude/ccam` links to the Windows one. See [WINDOWS-WSL.md](WINDOWS-WSL.md).

### `Input/output error (os error 5)` on `/home/<you>/.claude/ccam/accounts`
WSL lost its connection to the Windows drive, usually after the PC slept. Everything under `/mnt/c` fails, not just ccam. From PowerShell or cmd, reconnect it without restarting WSL:

```bat
wsl -d Ubuntu --cd ~ -u root -e sh -c "cd / && umount -l /mnt/c; mount -t drvfs 'C:\' /mnt/c -o uid=1000,gid=1000,noatime"
```

Then in any WSL shell that was already open, run `cd .`. `wsl --shutdown` also fixes it, but ends every WSL process including tmux sessions. Write `C:\` with the backslash and keep `uid=1000,gid=1000`: without them every file shows as owned by root and you get the next error.

### `Operation not permitted (os error 1)` on a `.ccam-tmp` file
The Windows drive is mounted without your user as owner (often after a remount that left out `uid=1000,gid=1000`). Current ccam tolerates this, but other tools will not. Remount as above.

### WSL `ccam list` shows its own account as `in use on WSL`
A record left from an older ccam under a different key. Current ccam removes these automatically on its next `list` or switch. If one stays, delete the stale file in `~/.claude/ccam/accounts/.live/`.

### WSL shows "stale" or "login expired" for accounts that work on Windows
WSL is reading its own old copy of the accounts instead of the shared one. Check `ls -la ~/.claude/ccam`: it should be a link to `/mnt/c/Users/<you>/.claude/ccam`. See [WINDOWS-WSL.md](WINDOWS-WSL.md).

## Locks and files

### `<path> is held by another process (waited <n>s)`
Another ccam, or Claude Code renewing its token, holds a lock. Retry in a few seconds. A lock left by a crashed process is taken over automatically after 60 seconds (Claude Code's) or when its holder stops refreshing it (ccam's).

### `lost the lock <path> (the machine may have slept); nothing further was written`
The machine slept while ccam held a lock, and something else took it over. ccam stopped before writing anything inconsistent. Run the command again.

### `<path>: invalid JSON: ...` / `<path>: <reason>`
A file ccam reads is damaged. For a file under `~/.claude/ccam/accounts/<name>/`, `ccam remove <name>` and save the account again. For `~/.claude.json` or `.credentials.json`, which belong to Claude Code, run `/login`.

### `macOS Keychain: ...`
The Keychain is locked or refused access. Unlock it (log in to the Mac) and retry. A locked Keychain is never treated as "logged out".

### `cannot determine the home directory`
`HOME` (or `USERPROFILE` on Windows) is not set in this environment.

## Automatic switching

### `ccam auto is already running (stop it with: ccam auto stop)`
Only one `ccam auto` runs per install (Windows and WSL each get their own). `ccam auto status` shows it; `ccam auto stop` stops it.

### `run ccam auto in its own terminal or tmux pane, not inside Claude Code`
`ccam auto` runs for hours. Give it its own terminal.

### It never switches
`ccam auto status` shows what it is waiting for:
- `watching`: usage is below the threshold.
- `armed`: close to the threshold, reading the other accounts.
- `over threshold, waiting out the switch cooldown`: switches are at least 15 minutes apart.
- `every account is nearly full`: it waits and names the account that reopens first.

Without `ccam setup --auto` it cannot hear when a turn ends, so it waits up to 10 minutes after crossing the threshold.

### No desktop notification
Run `ccam notify-test`. On Linux it needs `notify-send`; on macOS, Script Editor needs notification permission; on Windows (including WSL) it uses a PowerShell toast.

## Hooks

### A hook seems to do nothing
Hooks never print errors into Claude Code; failures go to `~/.claude/ccam/accounts/switch.log`. Check that file. `ccam setup` again is safe: it replaces ccam's hooks and leaves your own alone.

### I moved or reinstalled the ccam binary
The hooks point at the binary's full path. Run `ccam setup` again from the new binary.

## Reporting a bug

Open an issue with:
- what you ran and what it printed,
- the last 20 lines of `switch.log`,
- your OS, `ccam --version` and `claude --version`.

Never include tokens, the contents of `.credentials.json` or `login.json`, or email addresses.
