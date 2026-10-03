#!/usr/bin/env bash
# Installs the WSL helpers into ~/bin. Run inside WSL from the repository: bash scripts/install-wsl.sh
#   --windows   also write .cmd wrappers to %USERPROFILE%\bin, so the commands run from PowerShell and cmd
# Safe to rerun: it replaces its own files and never overwrites an existing ~/.tmux.conf.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin=$HOME/bin
mkdir -p "$bin"

for cmd in python3 tmux; do
    command -v "$cmd" >/dev/null || echo "note: $cmd is not installed (sudo apt install $cmd); some helpers need it"
done

# Copied with LF endings: a checkout on the Windows side may have CRLF, which bash cannot run.
install_one() { tr -d '\r' <"$here/$1" >"$bin/$2.new" && chmod +x "$bin/$2.new" && mv "$bin/$2.new" "$bin/$2"; }
install_one tm tm
install_one tmuxgo tmuxgo
install_one cmds cmds
install_one cresume.py cresume
for name in td tc tmuxdesktop tmuxclaude; do ln -sfn "$bin/tmuxgo" "$bin/$name"; done
tr -d '\r' <"$here/registry.txt" >"$bin/registry.txt"

if [[ -e $HOME/.tmux.conf ]]; then
    echo "kept your ~/.tmux.conf; to get the F1-F4 keys, add the lines from $here/tmux.conf"
else
    tr -d '\r' <"$here/tmux.conf" >"$HOME/.tmux.conf"
    echo "installed ~/.tmux.conf (F1 next pane, F2 zoom, F3 layout, F4 leave)"
fi

case ":$PATH:" in
    *":$bin:"*) ;;
    *) echo "add ~/bin to your PATH, e.g. in ~/.bashrc: export PATH=\"\$HOME/bin:\$PATH\"" ;;
esac

if [[ ${1:-} == --windows ]]; then
    profile=$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null </dev/null | tr -d '\r')
    winbin="$(wslpath "$profile")/bin"
    mkdir -p "$winbin"
    distro=${WSL_DISTRO_NAME:-Ubuntu}
    for name in tm td tc tmuxdesktop tmuxclaude cmds; do
        printf '@echo off\r\nwsl.exe -d %s -e %s/%s %%*\r\n' "$distro" "$bin" "$name" >"$winbin/$name.cmd"
    done
    if [[ -x $HOME/.cargo/bin/ccam ]]; then
        printf '@echo off\r\nwsl.exe -d %s -e %s/.cargo/bin/ccam %%*\r\n' "$distro" "$HOME" >"$winbin/csw.cmd"
    fi
    echo "wrote .cmd wrappers to $winbin; add that folder to your Windows PATH"
fi

echo "done. Try: cmds"
