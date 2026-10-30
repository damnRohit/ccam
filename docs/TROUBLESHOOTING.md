# Troubleshooting

Every message ccam prints when something is wrong, what it means, and how to fix it. Messages are quoted as ccam prints them; `<name>` stands for an account name.

**The universal fix.** Whatever state you are in, this works: run `/login` in Claude Code as the account you want, then `ccam save <name>`. It replaces the saved copy with a fresh login. Nothing else is lost.

**First look.** `ccam list` shows the saved accounts and which one is live. `~/.claude/ccam/accounts/switch.log` records every switch, sync and renewal with a timestamp. It holds no secrets and is safe to share.

## Contents

- [Logins and saved accounts](#logins-and-saved-accounts)
- [Adding an account with ccam login](#adding-an-account-with-ccam-login)
- [Switching](#switching)
- [Usage and start](#usage-and-start)
- [Lanes](#lanes)
- [Lanes in T3 Code](#lanes-in-t3-code)
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
The name is taken by another account. Pick another name, or `ccam remove <name>` first. `ccam login <name>` gives the same message when you signed in to an account other than the one saved as `<name>`.

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

## Adding an account with ccam login

`ccam login <name>` runs Claude Code's sign-in in a throwaway folder under `~/.claude/ccam/login/`, saves the login and deletes the folder. It never changes the main login, lanes or open sessions. `switch.log` records `login '<name>' (new)` or `login '<name>' (refreshed)`.

### `another ccam login is signing in; finish or cancel it first`
Another `ccam login` on this config home is still signing in, so this one stopped before touching anything; running both would let one delete the other's sign-in folder. Exit code 1. Finish or cancel the other sign-in (Ctrl+C in its terminal), then run `ccam login <name>` again. If no other sign-in is running, an earlier one crashed: its lock, `~/.claude/ccam/login.lock`, is taken over automatically a minute after the crash.

### `you signed in to '<x>', which is already saved, not to a new account; nothing was saved. Your browser was probably still signed in to it: retry in a private window, or run: ccam login <x> to refresh '<x>'`
The browser used its existing claude.ai session, so the sign-in was to an account ccam already has under another name. ccam saved nothing. Run `ccam login <name>` again and sign in in a private browser window (or sign out of claude.ai first), choosing the account you meant. To refresh the saved login of `<x>` instead, run `ccam login <x>`.

### `'<x>' is in use on <where>, so its login works and needs no new sign-in; nothing was changed`
`ccam login <x>` refreshes a saved login only when the account is in use nowhere. ccam checks this before the browser opens. `<where>` is `the main login`, `lane N`, `Windows`, `WSL (<distro>)` or `another machine`. Exit code 1. If the saved login still works, nothing needs refreshing. To replace it anyway, free the account first (`ccam to <other>`, `ccam move`, `ccam release`), then run it again.

### `'<x>' is in use on <where>, so its login works and was left alone; nothing was saved`
The same refusal, found after you signed in: the account became live while the browser was open. Exit code 1. Free it as above, then run `ccam login <x>` again.

### `the account you signed in to is in use on <where>, so its login works and was left alone; nothing was saved`
You signed in to an account that is not saved, but it is live in a lane or on your other install (`<where>` is `lane N`, `Windows`, `WSL (<distro>)` or `another machine`). Saving a second login for it would make the two copies disagree. Exit code 1. Run `ccam save <name>` from that install, or move that place to another account first and run `ccam login <name>` again. Or sign in as a different account.

### `the account you signed in to is the one the main login uses, and it is not saved; nothing was saved. Save it from there instead: ccam save <name>`
The account you signed in to is the one Claude Code is logged in to right now, and ccam has no copy of it. Exit code 1. Run `ccam save <name>`, as the message says. That saves the working login instead of a second one.

### `could not confirm which account you signed in to (<reason>); nothing was saved. Retry when online`
After the sign-in, ccam asks Anthropic which account the new token belongs to and saves nothing without an answer. The request failed (`<reason>` says why): you are offline, or Anthropic could not be reached. Exit code 3. Retry `ccam login <name>` when online. Unlike `ccam save`, `ccam login` never saves an unconfirmed login.

### `'<entry>' exists in the vault but cannot be read (<reason>); nothing was saved. Fix or delete <vault>/<entry> first`
The vault has a folder named `<entry>` under `~/.claude/ccam/accounts/` whose files do not load, and it may hold the only working copy of a login, so ccam will not write over it. Exit code 1. It is shown before the browser opens, and again after the sign-in if the entry broke in between. `ccam remove` cannot help, because it only finds accounts that load. Copy `login.json` somewhere safe if it may hold the only working token, then fix the file named in `<reason>` or delete the folder by hand, and run `ccam login <name>` again.

### `Claude Code reported a login but ccam cannot read it. It may have stored it in Windows Credential Manager, which ccam does not support yet. Nothing was saved`
The sign-in succeeded, but Claude Code kept the login somewhere ccam cannot read. Use `/login` in Claude Code as the account, then `ccam save <name>`. Nothing was saved and the throwaway folder is deleted.

### `the sign-in did not finish (claude exited with N); nothing was saved`
`claude auth login` stopped before it finished: you closed the browser tab, pressed Ctrl+C, or the sign-in failed (the `claude` output above this line says why). Run `ccam login <name>` again. `N` is Claude Code's exit code; 130 means the input closed before the code was pasted. If the browser cannot reach the machine (WSL over SSH, a container), paste the code the sign-in page shows into the terminal.

### `cannot start claude: ...; nothing was saved`
ccam could not run `claude`. Check that `claude --version` works in this terminal. Set `CCAM_CLAUDE_BIN` to the binary if it is not on `PATH`.

### `ccam: could not delete <path> (...); it holds a login, delete it by hand`
The throwaway folder, or Claude Code's `.lock` beside it, could not be removed (a file was open, or permissions). It may hold a working login. Close what uses it and delete it by hand. The next `ccam login` also tries to delete leftovers. ccam never runs `claude auth logout` on it: that would revoke the login the vault now holds.

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

## Lanes

Every lane event is in `switch.log`, prefixed `[lane <id>]`. `ccam sessions` shows what each lane holds and runs.

### `'<name>' is in lane <id>; using it here too would log one of them out.`
A lane holds that account. Either move the lane to another account (`ccam move <id> <other>`), or close its sessions and `ccam release <id>`. `--force` does not move lanes.

### `'<name>' is the main login; open it with plain `claude`...`
The main login is on that account, and one saved login cannot run in two places. Use `claude`, or move the main login first (`ccam to <other>`), then `ccam run <name>` (or `ccam lane <name>` for T3 Code). The same message comes from `ccam lane`.

### `ccam run starts Claude Code; run it from a normal terminal`
`ccam run` opens Claude Code in the current terminal. Run it from PowerShell or a shell, not from inside Claude Code.

### `this terminal is in lane <id>; open a new terminal, or unset CLAUDE_CONFIG_DIR first`
This shell's `CLAUDE_CONFIG_DIR` points at a lane. Open a fresh terminal, or clear the variable. `ccam run` and `ccam lane` both refuse here.

### `no lane <id>; see: ccam sessions`
No lane has that number or name. `ccam sessions` lists them. To create a named lane, give an account too: `ccam run <account> --lane <id>` or `ccam lane <account> --lane <id>`.

### `lane <id> holds no account; run: ccam run <account> --lane <id>`
The lane was released. Give it an account with the command shown (`ccam lane <account> --lane <id>` does the same without opening Claude).

### `lane <id> is on '<x>'; to change it, run: ccam move <id> <name>`
`ccam run <name> --lane <id>` or `ccam lane <name> --lane <id>` named a lane that holds another account. Neither changes a lane's account; `move` does, after asking.

### `'<name>' is in use by Claude on <side>; move <side> to another account first` (from `ccam move`)
The main login (or your other install) is on that account. Switch it first (`ccam to <other>`), then move the lane.

### `ccam move asks before moving; run it in a terminal, or add --yes`
`move` changes the account of every session in the lane, so it asks first. Without a terminal to ask in (scripts, `!` inside Claude Code), add `--yes`.

### `lane <id> has <n> running session(s) ...; close them first, or run: ccam release <id> --force`
Releasing logs the lane out, and its open sessions with it. Close them (see `ccam sessions`), or use `--force` to release anyway.

### `cannot list lane <id>'s sessions (...)`
`claude agents --json` failed, so ccam cannot tell whether sessions run in the lane. Close them yourself, then `ccam release <id> --force`.

### `lanes need Claude Code 2.1.296 or later`
Older Claude Code can delete the shared history through a lane's junction during cleanup. Run `claude update`.

### `invalid lane ...`
A lane is a number from 1 without leading zeros, or a name that starts with a letter or digit, is not only digits, and uses letters, digits, `.`, `_`, `@` or `-`.

### `cannot link to ...` (from `ccam run`)
ccam could not create a junction (Windows) or symlink under `~/.claude/ccam/lanes/<id>`. Check that the drive allows it and that the target named in the message exists.

### Claude asks before every memory write in a lane
`ccam run` points memory at the main login's folder so Claude Code does not ask. If `switch.log` has `memory not redirected` for that run, the project's folder name is over 200 characters and ccam could not find its folder: start one session in that project with plain `claude` first, then `ccam run` again. A lane opened another way (setting `CLAUDE_CONFIG_DIR` by hand) has no redirect.

### A session in a lane shows the wrong account, or asks for `/login`
Check `switch.log` for that lane's lines: the `run`, any `switch` after it, and `sync` lines from renewals. A `[lane <id>] release` while the session ran logs it out; run `ccam run <account> --lane <id>` again. Include those lines when reporting a bug.

## Lanes in T3 Code

`ccam lane`, `ccam run`, `move`, `release` (also with `--delete`) and `prune` keep one T3 Code instance in step with each lane. ccam edits only the `ccam-lane-*` entries in `~/.t3/userdata/settings.json` (or `$T3CODE_HOME/userdata/settings.json`), and sets only `driver`, `displayName`, `enabled` and `config.homePath` in them. Every other entry and key stays as it was.

`ccam lane work` prints these lines when it works:

```
Lane 1 holds 'work'.
T3 Code: lane 1 is in the model picker as 'Claude · lane 1 · work'.
```

### T3 does not show the lane
1. Run `ccam sessions`. The lane must exist and hold an account. A lane with no account has its instance disabled, so T3 hides it from the model picker.
2. Open `~/.t3/userdata/settings.json` and look under `providerInstances` for the lane's instance (see the instance id entry below). Its `displayName` reads `Claude · lane <id> · <account>` and `config.homePath` is the lane's folder under `~/.claude/ccam/lanes/`. If the entry is missing, run `ccam lane <account> --lane <id>` again.
3. If T3 keeps its data somewhere else, `T3CODE_HOME` is set. ccam reads it too, but only from the shell it runs in, and an empty value counts as unset. Set it the same way for ccam and for T3, or unset it.
4. If ccam printed `T3 Code not found; skipped adding the lane to T3.`, there is no `~/.t3/userdata` folder (or `$T3CODE_HOME/userdata`) yet. Start T3 once, then run `ccam lane <account> --lane <id>` again.
5. If ccam printed `ccam: could not update T3 Code: ...`, see that entry below.
6. T3 reloads the file within a second, but if the entry is in the file and the picker still lacks it, restart T3.

### `T3 Code not found; skipped adding the lane to T3.`
Printed by `ccam lane` on stdout. The lane was created and holds the account. ccam found no `userdata` folder in `$T3CODE_HOME`, or in `~/.t3` when `T3CODE_HOME` is unset, so it treats T3 Code as not installed and does not create the folder. `ccam run`, `move`, `release` and `prune` skip T3 without a message. Start T3 Code once (it creates `userdata`), or set `T3CODE_HOME` to where it keeps its data, then run `ccam lane <account> --lane <id>` again.

### `ccam: could not update T3 Code: <path>: <reason>`
Printed on stderr. The command itself succeeded: the lane exists and holds its account, or was moved or released. Only the T3 entries were not updated, and the line `t3: <path>: <reason>` is in `switch.log`. Nothing in T3's file was written for this step. The next `ccam lane`, `run`, `move`, `release` or `prune` retries, so after fixing the cause, `ccam lane <account> --lane <id>` brings T3 up to date. What to check, by `<reason>`:
- `providerInstances is not a JSON object` or `top level is not a JSON object`: T3's `settings.json` has a shape ccam will not edit. Fix the file by hand or let T3 repair it.
- `invalid JSON: ...`: the file does not parse. T3 blocks its own saves on such a file as well. Fix it, or restore your copy.
- `T3 Code kept overwriting the lane instances; try again`: see below.
- An I/O error (`Permission denied`, `Access is denied`, file locked): the file or its folder cannot be written. Check its permissions and that nothing holds it open.
- An error naming a path under `~/.claude/ccam/lanes/`: ccam could not read that lane's login state. Run `ccam sessions` to see which lane, and the troubleshooting entries for [Lanes](#lanes).

### `<path>: providerInstances is not a JSON object` / `<path>: top level is not a JSON object` / `<path>: invalid JSON: ...`
T3's `settings.json` has a shape ccam will not change, or does not parse. ccam wrote nothing. Fix the file by hand or let T3 repair it, then run the command again. These appear after `ccam: could not update T3 Code: `.

### `<path>: T3 Code kept overwriting the lane instances; try again`
ccam writes the file, reads it back, and writes once more if T3 saved its own settings in between and dropped the edit. T3 did that twice in a row. Run the command again.

### The instance id is not `ccam-lane-<id>`
T3 accepts instance ids of up to 64 characters that start with a letter and use only letters, digits, `_` and `-`. A lane id made only of ASCII letters, digits and `-` keeps its spelling: lane `1` is `ccam-lane-1`, lane `Work-2` is `ccam-lane-Work-2`. Any other lane id (one with `.`, `_` or `@`, or one too long) is sanitised: every other character becomes `_`, the id is shortened to fit, and `_` plus the first 8 hex characters of a SHA-256 hash of the lane id are added. Lane `a.b` becomes `ccam-lane-a_b_` followed by 8 hex characters. The hash keeps lanes that sanitise alike apart. Clean ids never contain `_`, so a hashed id cannot be mistaken for one. Look for the `displayName`, `Claude · lane <id> · <account>`, when you need to find a lane's entry in the file.

### A lane's `ccam-lane-*` entry disappeared, or my edit to it was overwritten
ccam owns `ccam-lane-*` entries. It removes the entry of a lane whose folder is gone, and resets `driver`, `displayName`, `enabled` and `config.homePath` on the rest at every sync. Other keys you add to such an entry (`accentColor`, `environment`, `config.launchArgs`) are kept. Entries without the `ccam-lane-` prefix are never touched, so copy an entry under another id if you want settings of your own.

### A thread in a lane instance asks before every memory write
T3 threads run in full-access mode, which should not ask. If yours does, the cause is anthropics/claude-code#98044 (see the Lanes entry on memory). Approving the prompt works, and the note is still saved to the main login's memory folder. Please report it with the result of step 1 in [MANUAL-TESTS.md](MANUAL-TESTS.md) section 10.

### A thread lost its conversation after I changed its instance in T3
Changing a thread's instance in T3 starts a fresh Claude session and gives it about 16k tokens of history. To follow an account that ran out, keep the thread where it is and run `ccam move <lane> <other>`: the thread keeps its conversation and rebuilds its prompt cache once.

### A child agent ran on another account
A child agent started without a target uses its parent's instance. One started with only a driver (`claudeAgent`) can land on another healthy Claude instance. Name the parent's instance when delegating, or turn off the other Claude instances for the project in T3's provider settings.

### The lane's settings are out of date in T3
A lane keeps copies of the main `settings.json` (with ccam's hooks for the lane), `CLAUDE.md` and `keybindings.json`. Its session-start hook refreshes them and writes a file only when it changed, so an edit reaches the next session in the lane, not one already running. Start a new thread on the instance. `switch.log` has `[lane <id>] settings refreshed` after each session start that worked. If it has `[lane <id>] settings not refreshed: <reason>` instead, the copies are as they were; the reason names the file (for example `<path> is not inside a main config home`, or `top level is not a JSON object` for a main `settings.json` that is not a JSON object). Fix that, or run `ccam lane <account> --lane <id>`, which refreshes the copies too.

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
A file ccam reads is damaged. For a file under `~/.claude/ccam/accounts/<name>/`, delete that folder by hand (copy `login.json` somewhere safe first if it may hold the only working token; `ccam remove` cannot find an entry that fails to load), then `ccam save <name>` or `ccam login <name>` again. For `~/.claude.json` or `.credentials.json`, which belong to Claude Code, run `/login`.

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
