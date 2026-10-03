# WSL helpers

Optional shell tools for running Claude Code in WSL, at the PC or from a phone over SSH. ccam does not need any of them. Same caveat as the rest of the repository: vibe-coded, used daily by one person, use at your own risk.

| Command | What it does |
|---|---|
| `tc <folder>` | Claude Code in a Desktop folder, inside a tmux session that keeps running when you close the terminal |
| `td <folder>` | A plain shell in a Desktop folder, in a tmux session |
| `tm` | List tmux sessions by number; open, show several at once, or remove them |
| `cresume` | Pick a Claude conversation to resume, including ones started on Windows |
| `cmds` | One list of all these commands with examples; `cmds <name>` for one command's help |

Every command has `-h`. `tm help` is a full guide to tmux keys, switching and phone use.

## Install

Inside WSL, from the repository:

```sh
bash scripts/install-wsl.sh            # copies into ~/bin, installs ~/.tmux.conf if you have none
bash scripts/install-wsl.sh --windows  # also writes .cmd wrappers to %USERPROFILE%\bin for PowerShell and cmd
```

Needs `tmux` and `python3` (`sudo apt install tmux python3`). Rerun it after pulling changes.

## Examples

```sh
tc pix          # Claude Code in ~/Desktop/pixel-thing (partial, any case); reattaches if already running
tc -c pix       # same, continuing the last conversation
tc -r pix       # same, picking a conversation to resume
td rsr/ana      # a shell in Desktop/rsrSomething/analytics
tm              # numbered list of sessions
tm 2            # open session 2
tm 1 3          # sessions 1 and 3 on one screen: side by side, or up/down on a narrow phone screen
tm rm 3         # remove session 3
cresume -a      # latest conversations of every project
```

Folder names match exactly first, then by prefix, then anywhere in the name, ignoring case. When a partial name matches several folders, the command lists them instead of guessing.

## Keys inside tmux

| Key | Does |
|---|---|
| F1 | next pane |
| F2 | zoom the pane to full screen, and back |
| F3 | next layout (side by side, up/down, grid) |
| F4 | leave; everything keeps running |
| `Ctrl+b` then `s` | pick a session from a list |
| `Ctrl+b` then `d` | leave (same as F4) |

The F keys come from `tmux.conf` and exist because phone keyboards make `Ctrl+b` chords awkward. Termius has F1 to F12 on its extra key row. Termius does not pass taps to tmux, so tapping a pane does nothing there; use F1.

## Keeping sessions alive

WSL shuts a distro down about 15 seconds after its last window closes, and every tmux session dies with it. `tm start` keeps WSL running with no window open (a hidden `wsl.exe`, started through WMI so it also survives an SSH disconnect); `tm stop` undoes it. `tc`, `td` and multi-session views run `tm start` for you. The first line of `tm` says whether it is on.

The permanent alternative is `instanceIdleTimeout=-1` in `%UserProfile%\.wslconfig`; see [docs/WINDOWS-WSL.md](../docs/WINDOWS-WSL.md).

## Conversations from Windows

Claude Code files each folder's conversations under `~/.claude/projects/<key>`, and Windows and WSL compute different keys for the same folder. `cresume` links every Windows project folder into WSL (once per new folder; existing WSL folders are never touched) and lists conversations from both sides, newest first. It resumes by ID because WSL's `/resume` skips linked folders.

`link-windows-sessions.sh` does only the linking: `--dry-run` to preview, `--undo` to remove the links.

Do not open the same conversation on Windows and WSL at once; both would write to one file.

## Adding your own command to `cmds`

`cmds` reads `registry.txt`, one line per command: `group | name | where it runs | what it does | examples separated by ;`. Add a line, and `cmds` shows it. `cmds --check` lists commands in `~/bin` (and the Windows bin folders) that have no line yet.

## Assumptions

- Projects live in your Windows Desktop folder (found through PowerShell once, then cached in `~/.cache/tmuxgo-desktop`).
- The Windows drive is `C:` mounted at `/mnt/c`.
- The distro name comes from `WSL_DISTRO_NAME`, falling back to `Ubuntu`.
