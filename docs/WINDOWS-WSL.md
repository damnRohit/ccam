# Windows and WSL on one PC

How to run Claude Code on Windows and in WSL with one set of saved accounts, what can go wrong, and how to fix it. Everything here was worked out on Windows 11 with WSL 2 and Ubuntu.

## Why sharing is needed

Claude Code on Windows and Claude Code in WSL are separate installs: `C:\Users\<you>\.claude` and `/home/<you>/.claude`, each with its own live login.

A login's refresh token works once. When Claude Code renews a login, it gets a new token and the old one dies. So if Windows and WSL each keep their own copy of your saved accounts, every renewal on one side kills the other side's copy of that account. You see this as `stale login` and `login expired` on one side for accounts that work fine on the other.

The fix has two parts:

1. **One shared set of saved accounts.** WSL's `~/.claude/ccam` is a link to the Windows folder, so whichever side renews a login saves it where the other side also reads it.
2. **Each account live on one side at a time.** If both sides were logged in to the same account from the same saved login, the first renewal would log the other out. ccam enforces this rule for you.

## Setup

1. Install ccam on Windows (PowerShell) and in WSL, both from the same source:
   ```sh
   cargo install --locked --path .
   ccam setup
   ```
2. Save your accounts on Windows (see the README's quick start).
3. In WSL, replace WSL's own folder with a link to the Windows one:
   ```sh
   mv ~/.claude/ccam ~/ccam-wsl-backup 2>/dev/null
   ln -s "/mnt/c/Users/<you>/.claude/ccam" ~/.claude/ccam
   ccam list
   ```
   `ccam list` in WSL should now show the Windows accounts. Usage works for all of them.
4. Put each side on a different account. If WSL's own login is blank or stale, just switch it: `ccam to <name>` works from a logged-out state, no `/login` needed.

From Windows you can also manage WSL's account by running WSL's ccam, for example with a `csw.cmd` in your PATH:

```bat
@echo off
wsl.exe -d Ubuntu -e /home/<you>/.cargo/bin/ccam %*
```

Then `csw list`, `csw usage` and `csw to <name>` act on WSL's Claude from PowerShell.

## How the one-side rule works

Each install writes a small record to `~/.claude/ccam/accounts/.live/<key>.json` saying which account its Claude is logged in as and how to run its ccam. Records are updated on every switch, save and sync. The hooks run sync on login and token renewal, so records follow Claude Code even when you did not use ccam.

- `ccam list` marks accounts the other side uses: `in use on WSL (Ubuntu)` or `in use on Windows`.
- `ccam to X` refuses an account the other side uses.
- `ccam to X --force` first checks this side can switch, then runs the other side's ccam (`wsl.exe` from Windows, `ccam.exe` from WSL) to move it to the free account with the most room, then switches here.
- `ccam auto` never targets the other side's account, and `usage --refresh` and `start --refresh` never renew it.
- Each side has its own `ccam auto` (its own lock, signals and status), so a limit hit in WSL never triggers a switch on Windows.

The check and the record are both made under the shared vault lock, so two sides switching at the same moment cannot both get the same account.

## Things that go wrong

### WSL loses the Windows drive: `Input/output error`
After the PC sleeps, WSL's connection to `C:` sometimes dies. Everything under `/mnt/c` fails with `Input/output error (os error 5)`, and so does ccam, because the shared accounts live on `C:`. Windows itself is fine.

Reconnect from PowerShell or cmd, without restarting WSL (tmux sessions survive):

```bat
wsl -d Ubuntu --cd ~ -u root -e sh -c "cd / && umount -l /mnt/c; mount -t drvfs 'C:\' /mnt/c -o uid=1000,gid=1000,noatime"
```

Then run `cd .` in shells that were already open. It must run from Windows: inside WSL the fix needs root, and the Windows tools WSL would use live on the broken drive.

Two details matter. Write the drive as `C:\` (with the backslash), and keep `uid=1000,gid=1000`. Without them, every file on `C:` shows as owned by root, and tools that set file permissions fail with `Operation not permitted`. Saving a login still works in current ccam, but other tools will not.

### WSL shows the wrong time zone
WSL follows Windows' time zone, but Windows' "India Standard Time" maps to the old name `Asia/Calcutta`, which Ubuntu 24.04 and later moved into the `tzdata-legacy` package. Without it WSL falls back to UTC. Fix it once:

```sh
sudo apt install tzdata-legacy
```

Other zones with legacy names can hit the same problem; the fix is the same.

### tmux sessions die when you close the last WSL window
WSL stops a distro about 15 seconds after its last `wsl.exe` session ends (`instanceIdleTimeout`, default 15000 ms), and every tmux session dies with it, even ones started over SSH. Two fixes:

- Global and permanent: `%UserProfile%\.wslconfig` with
  ```ini
  [general]
  instanceIdleTimeout=-1
  ```
  then `wsl --shutdown` once.
- On demand: keep one hidden `wsl.exe` running. `tm start` in `scripts/` does this (see [scripts/README.md](../scripts/README.md)).

### `/resume` in WSL does not show Windows conversations
Claude Code stores conversations under `~/.claude/projects/<folder key>`, and Windows and WSL use different keys for the same folder (`C--Users-...` and `-mnt-c-Users-...`). Linking WSL's folders to the Windows ones shares the history, but WSL's `/resume` skips linked folders. Resuming by ID works: `claude --resume <id>`. `scripts/cresume.py` lists the conversations and resumes by ID. Do not open the same conversation on both sides at once; both would write to one file.

### A side was uninstalled and its record blocks an account
`ccam to X` keeps saying X is in use on a side that no longer exists. The error from `--force` names the record file in `~/.claude/ccam/accounts/.live/`. Delete it.

### Windows' `.claude` folder is readable by other local groups
Saved logins are plain JSON protected only by file permissions, as Claude Code's own credentials file is. Check who can read `C:\Users\<you>\.claude` with `icacls "%USERPROFILE%\.claude"` and remove groups that should not read it.

## Undoing it

In WSL: `rm ~/.claude/ccam` (removes only the link), then `mv ~/ccam-wsl-backup ~/.claude/ccam` to get WSL's old copy back. Its logins are probably stale by then; `/login` and `ccam save` refresh them.
