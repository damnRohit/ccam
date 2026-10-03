# Shell helpers: numbered tmux session picker and a personal command help

Researched 2026-09-25 against the tmux man page and source, the sesh, tmux-sessionizer and tmux-sessionx READMEs and source, the tldr-pages style guide, tealdeer docs, navi docs, the cheat/cheat README, and the cheat.sh README. Local machine: WSL, `tmux 3.6`.

## Summary

- tmux already has everything a numbered picker needs: `list-sessions -F` for machine-readable rows, `has-session` for existence checks, `attach-session` outside tmux, `switch-client` inside, and `kill-session`. The picker is a thin shell function, not a tool to install.
- Always target sessions as `=name`. Without `=`, tmux accepts a name prefix or glob, so `kill-session -t analytics-w` kills `analytics-web`. Verified locally on tmux 3.6.
- Branch on `$TMUX`: empty means attach, set means switch-client. Both sessionizer tools do exactly this.
- For help, a small registry file read by one `help` function beats installing tldr/navi/cheat for ~10 commands. Copy the tldr page shape: one-line description, then 3 to 5 "imperative description, then example" pairs.

## Local grounding

- `tmux -V` in WSL: `tmux 3.6`. No server was running at check time (`error connecting to /tmp/tmux-1000/default`).
- Probe on an isolated server (`tmux -L probe`), sessions `analytics` and `analytics-web`:
  - `has -t analy` exits 1 (ambiguous prefix matches two sessions).
  - `has -t =analy` exits 1 (exact match only).
  - `kill-session -t analytics-w` exits 0 and kills `analytics-web` by prefix.
  - `new -ds my.proj:x` creates a session named `my_proj_x`.

## 1. tmux session pickers

### Built-in commands

| Command | What it does | Source |
|---|---|---|
| `list-sessions [-F format] [-f filter]` (alias `ls`) | Lists sessions, one line per session, in the given format. tmux 3.6 has only `-F` and `-f`; newer master adds `-O` sort and `-r` | [tmux.1 master](https://github.com/tmux/tmux/blob/master/tmux.1), [tmux.1 3.6](https://github.com/tmux/tmux/blob/3.6/tmux.1) |
| Format vars | `#{session_name}`, `#{session_windows}`, `#{session_attached}` (number of clients attached), `#{session_activity}`, `#{session_created}` | [tmux.1 FORMATS](https://github.com/tmux/tmux/blob/master/tmux.1) |
| `has-session -t target` (alias `has`) | Exit 1 if the session does not exist, 0 if it does | [tmux.1](https://github.com/tmux/tmux/blob/master/tmux.1) |
| `attach-session -t target` (alias `attach`) | From outside tmux, attach in the current terminal. From inside, switch the current client. Target must exist; `new-session -A` creates or attaches | [tmux.1](https://github.com/tmux/tmux/blob/master/tmux.1) |
| `switch-client -t target` (alias `switchc`) | Switch the current client to another session. `-l`/`-n`/`-p` go to last, next, previous | [tmux.1](https://github.com/tmux/tmux/blob/master/tmux.1) |
| `kill-session -t target` | Destroy the session and detach all clients attached to it. `-a` kills all but the target | [tmux.1](https://github.com/tmux/tmux/blob/master/tmux.1) |
| `choose-tree -s` | Interactive tree of sessions with a shortcut key per row, Enter to choose, `x` to kill, `C-s` to search. Works only if a client is attached | [tmux.1](https://github.com/tmux/tmux/blob/master/tmux.1) |
| `choose-session` | Now just an alias of `choose-tree` via the `command-alias` option | [tmux CHANGES](https://github.com/tmux/tmux/blob/master/CHANGES) |

Default prefix keys (prefix is `C-b`): `d` detach the current client, `s` select a session interactively, `w` choose a window, `$` rename the session, `(` and `)` previous and next session ([tmux.1 DEFAULT KEY BINDINGS](https://github.com/tmux/tmux/blob/master/tmux.1)).

Target resolution for `target-session`, in order: `$id`, exact name, start of a name, glob. A leading `=` accepts only an exact match. Multiple matches produce an error ([tmux.1 COMMANDS](https://github.com/tmux/tmux/blob/master/tmux.1)).

### sesh

- Go CLI that lists tmux sessions, zoxide directories, and configured sessions from `sesh.toml`, and connects to one, creating it if needed. Typical use is `sesh connect $(sesh list | fzf)` or a tmux popup binding ([README](https://github.com/joshmedeski/sesh)).
- Naming: session name from the directory basename by default, `dir_length` to use more components, `name_substitution` rules to rewrite paths. README notes tmux names cannot contain `.` or `:` and spaces become `_` ([README](https://github.com/joshmedeski/sesh#session-name-substitution)).
- Attach vs switch: `tmux/tmux.go` calls `attach-session` or `switch-client` and decides by `len(os.Getenv("TMUX")) > 0` ([source](https://github.com/joshmedeski/sesh/blob/main/tmux/tmux.go)).
- Kill: the fzf example binds `ctrl-d` to `tmux kill-session -t {2..}` then reloads the list. The built-in picker allows killing the attached session: "tmux moves your client to another session, or exits if that was the last one" ([README](https://github.com/joshmedeski/sesh)). It recommends `set -g detach-on-destroy off` and ships `sesh last` because prefix `L` breaks after a session is closed ([README](https://github.com/joshmedeski/sesh#recommended-tmux-settings)).
- Display vs value: when using `--format`, keep the raw session name in a hidden tab-separated field, because formatted rows "are not parsed back by `sesh connect`" ([README](https://github.com/joshmedeski/sesh#custom-list-formatting)).

### ThePrimeagen's tmux-sessionizer

- Bash script: fzf over a set of directories plus existing sessions (prefixed `[TMUX] `), then creates a detached session if missing and switches to it ([README](https://github.com/ThePrimeagen/tmux-sessionizer), [script](https://github.com/ThePrimeagen/tmux-sessionizer/blob/master/tmux-sessionizer)).
- Naming: `basename "$selected" | tr . _`.
- `switch_to()`: `attach-session` when `$TMUX` is empty, else `switch-client`.
- Existence check is inconsistent: one place uses `has-session -t="$target"` (exact), but the main path uses `tmux list-sessions | grep -q "^$1:"`, and `switch-client -t "$1"` has no `=`. Fine for its own basename names, a trap to copy.
- Optional per-directory `.tmux-sessionizer` file is sourced into the new session via `send-keys`.

### tmux-sessionx

- TPM plugin with an fzf popup: preview, `Ctrl-r` rename, `Ctrl-w` windows mode, `Ctrl-t` native tree preview, plus delete, zoxide and tmuxinator sources. Needs tpm, fzf, fzf-tmux, bat ([README](https://github.com/omerxx/tmux-sessionx)).
- Runs only from inside tmux, so it does not cover "pick a session from a plain shell".

### Conventions worth copying

- One name per directory, derived from the folder basename, sanitized (`.` and `:` to `_`). This makes `td <folder>` idempotent: `has-session -t "=$name" || new-session -ds "$name" -c "$dir"`, then attach or switch.
- Machine rows from `ls -F '#{session_name}'`, pretty rows only for display.
- Show attached state (`#{session_attached}`) and window count next to the number.
- Remind the user that `Ctrl+b d` detaches without killing, and `Ctrl+b s` is the in-tmux picker.

### Pitfalls

- **Prefix matching.** `-t name` without `=` can hit a different session by prefix or glob. Use `-t "=$name"` everywhere, especially for `kill-session`.
- **Nested tmux.** `attach-session` from inside a pane fails with "sessions should be nested with care, unset $TMUX to force" ([cmd-attach-session.c](https://github.com/tmux/tmux/blob/master/cmd-attach-session.c)). Use `switch-client` when `$TMUX` is set.
- **Killing the attached session.** With the default `detach-on-destroy on`, the client is detached (your terminal drops back to the shell). With `off`, it moves to the most recently active remaining session ([tmux.1 detach-on-destroy](https://github.com/tmux/tmux/blob/master/tmux.1)). Warn or confirm before killing the session you are in.
- **Unstable numbers.** Numbers shift after a kill. Resolve number to name immediately before acting, and accept names too.
- **No server.** `tmux ls` exits non-zero with "error connecting" when no server runs. Treat that as "no sessions", not an error.
- **`choose-tree` needs an attached client.** It is not usable from a plain shell.

## 2. Personal command registry and help

### tldr pages

- Markdown page per command: `# name`, `> ` description lines (plus optional `> More information: <url>.`), then `- Imperative description:` followed by a backticked example, with `{{placeholders}}` for user values ([style guide](https://github.com/tldr-pages/tldr/blob/main/contributing-guides/style-guide.md)).
- Keep pages around 5 examples, never more than 8 ([CONTRIBUTING](https://github.com/tldr-pages/tldr/blob/main/CONTRIBUTING.md)).
- Custom pages: tealdeer reads `<command>.page.md` from its custom pages dir (`~/.local/share/tealdeer/pages/` on Linux, `tldr --show-paths`) and shows them for `tldr <command>` ([tealdeer docs](https://github.com/tealdeer-rs/tealdeer/blob/main/docs/src/usage_custom_pages.md)).

### navi

- `.cheat` or `.cheat.md` files. `%` starts a cheat with tags, `#` is a description, the next line is the command, `<var>` marks a placeholder, and `$ var: shell command` supplies suggestions fed to fzf. `;` lines are comments ([syntax](https://github.com/denisidoro/navi/blob/master/docs/cheatsheet/syntax/README.md)).
- Personal files go under `$(navi info default-cheats-path)` ([cheatsheet docs](https://github.com/denisidoro/navi/blob/master/docs/cheatsheet/README.md)). Runs as a command or a Ctrl-R style shell widget ([README](https://github.com/denisidoro/navi)).
- Strength: `$ session: tmux ls -F '#{session_name}'` gives live completion. Cost: fzf dependency and a separate syntax.

### cheat (cheat/cheat)

- Plain-text files with no extension, named after the command (`cheat tar` reads `tar`), optional YAML front matter for `syntax` and `tags`. `cheat -e name` edits or creates, `cheat -l` lists names and tags ([README](https://github.com/cheat/cheat)).
- Multiple "cheatpaths" in `conf.yml`, for example a read-only `community` path and a `personal` path with its own tags ([README](https://github.com/cheat/cheat#cheatpaths)).

### cheat.sh

- Web service: `curl cht.sh/tar`, `curl cht.sh/go/:list`, and free-text queries like `curl cht.sh/python/random+list+elements`. It aggregates cheat.sheets, tldr, cheat, learnxinyminutes, and StackOverflow; self-hosting via Docker exists ([README](https://github.com/chubin/cheat.sh)).
- It is for community knowledge, not your own commands.

### Plain registry plus a `help` lister

- One file (TSV or a small shell array) with `name`, `summary`, `example` rows, sourced by one function that prints an aligned table, and `help <name>` prints that command's examples.
- No dependency, lives next to the functions it documents, and cannot drift if each function's entry sits right beside its definition.

### What is worth copying for ~10 commands

- tldr's shape: one-line summary, imperative example descriptions, `{{placeholder}}` or `<placeholder>` values, at most about 5 examples.
- cheat's "file named after the command" idea, or simply one registry file.
- navi only if live completion of arguments matters. It is heavier than the problem.
- Skip cheat.sh for this purpose.

## Recommendation for our case

1. **Session command `ts` (a single function, WSL bash).**
   - `ts` or `ts ls`: `tmux ls -F '#{session_name}|#{session_windows}|#{session_attached}'`, sort by name, print `1) analytics  3 win  (attached)`. Treat "no server" as "no sessions". Print a footer: `Ctrl+b d detach, Ctrl+b s switch inside tmux`.
   - `ts <n|name>`: resolve number to name from a fresh listing, check `has-session -t "=$name"`, then `attach-session -t "=$name"` if `$TMUX` is empty, else `switch-client -t "=$name"`.
   - `ts rm <n|name>`: resolve, confirm when it is the current session (`tmux display -p '#S'` inside tmux), then `kill-session -t "=$name"`.
   - Reuse the same resolver in `td`/`tc` so all tmux commands share one naming rule (folder basename, `.` and `:` to `_`) and one attach-or-switch helper.
   - Leave `detach-on-destroy` at its default unless you want `ts rm` of the current session to hop to another one. If so, set it to `off` in `.tmux.conf`.
2. **Global help command `cmds` (avoid shadowing bash's builtin `help`).**
   - One registry file, for example `~/.config/rsr/commands.tsv`, with rows `name<TAB>summary<TAB>example` and several rows per command for multiple examples. Cover `td`, `tc`, `ts`, `cresume`, `ccam`, `cs`.
   - `cmds` prints `name  summary` aligned. `cmds <name>` prints tldr-style examples for that command.
   - Write summaries and examples in tldr style: imperative, 1 to 5 examples each, `<placeholders>` for arguments.
   - Add a check (a one-line test or `cmds --check`) that every function defined in your helpers file has a registry row, so the list cannot drift.
3. Do not install sesh, tmux-sessionizer, tmux-sessionx, navi or cheat for this. They solve fuzzy picking across many directories or community cheat sheets. Your need is about 10 known commands and a numbered list, and the built-in commands cover it.
