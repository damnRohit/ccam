#!/usr/bin/env bash
# Run inside WSL. Makes Claude Code in WSL see (and add to) the conversation history of Claude Code on Windows.
#
# Claude Code files conversations under ~/.claude/projects/<working dir with separators turned into dashes>.
# Windows and WSL have separate ~/.claude folders, and the same project has two different paths:
#   Windows  C:\Users\me\proj   ->  c--Users-me-proj
#   WSL      /mnt/c/Users/me/proj  ->  -mnt-c-Users-me-proj
# This links each WSL project folder to its Windows twin, so both write to the same history. WSL's /resume skips
# linked folders (the same files in a real or bind-mounted folder are listed); cresume.py lists them and resumes
# by id.
#
# Safe to rerun: existing correct links are kept, and a real WSL folder that already exists is never touched
# (it is reported so you can merge it by hand). --dry-run shows what would happen. --undo removes the links.
set -euo pipefail

mode=link
case "${1:-}" in
  --dry-run) mode=dry ;;
  --undo) mode=undo ;;
  "") ;;
  *) echo "usage: $0 [--dry-run|--undo]" >&2; exit 2 ;;
esac

win_user_profile=$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')
win_projects=$(wslpath "$win_user_profile")/.claude/projects
wsl_projects="$HOME/.claude/projects"
[ -d "$win_projects" ] || { echo "no Windows Claude history at $win_projects" >&2; exit 1; }
mkdir -p "$wsl_projects"

linked=0 kept=0 skipped=0
for win_dir in "$win_projects"/*/; do
  key=$(basename "$win_dir")
  # Only drive-letter projects map to /mnt/<drive>: "c--Users-..." -> "-mnt-c-Users-..."
  [[ "$key" =~ ^([A-Za-z])--(.*)$ ]] || continue
  drive=${BASH_REMATCH[1],,}
  wsl_key="-mnt-${drive}-${BASH_REMATCH[2]}"
  target="$wsl_projects/$wsl_key"

  if [ "$mode" = undo ]; then
    if [ -L "$target" ]; then rm "$target"; echo "unlinked $wsl_key"; fi
    continue
  fi
  if [ -L "$target" ]; then
    if [ "$(readlink "$target")" = "${win_dir%/}" ]; then kept=$((kept + 1)); continue; fi
    echo "skip  $wsl_key: links somewhere else ($(readlink "$target"))"; skipped=$((skipped + 1)); continue
  fi
  if [ -e "$target" ]; then
    echo "skip  $wsl_key: WSL already has its own sessions here; move them into ${win_dir%/} by hand to merge"
    skipped=$((skipped + 1)); continue
  fi
  if [ "$mode" = dry ]; then
    echo "would link $wsl_key -> ${win_dir%/}"
  else
    ln -s "${win_dir%/}" "$target"
  fi
  linked=$((linked + 1))
done

[ "$mode" = undo ] && exit 0
verb=$([ "$mode" = dry ] && echo "would link" || echo "linked")
echo "$verb $linked, already linked $kept, skipped $skipped"
echo "WSL's /resume skips linked folders. Use cresume from the project folder (or cresume -a)."
