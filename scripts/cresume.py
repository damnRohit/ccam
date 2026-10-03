#!/usr/bin/env python3
"""Pick and resume a Claude Code session in WSL, including ones started on Windows.

WSL shares Windows history by linking each ~/.claude/projects/<folder> to its Windows twin, but Claude's
/resume skips linked folders (tested with Claude Code 2.1.281: a linked folder lists nothing, the same files
in a real or bind-mounted folder list fine). `claude --resume <id>` opens the file directly, so this lists
sessions itself and resumes by id. Every run first links Windows project histories that have no WSL twin yet
(what link-windows-sessions.sh does), so new Windows projects show up without extra steps.

Usage: cresume [-a] [count] [-- extra claude args]
  cresume          chats of this folder, newest first; type a number to resume one
  cresume -a       latest chats of every project; resumes in that chat's own folder
  cresume 50       show 50 instead of 20
  cresume -- --model sonnet    pass options on to claude
"""
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

PROJECTS = Path.home() / ".claude" / "projects"
CACHE = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "cresume.json"


def project_key(path):
    return re.sub(r"[^A-Za-z0-9]", "-", path)


def windows_projects():
    try:
        profile = subprocess.run(["cmd.exe", "/c", "echo %USERPROFILE%"], capture_output=True, text=True,
                                 stdin=subprocess.DEVNULL, cwd="/", timeout=10).stdout.strip()
        home = subprocess.run(["wslpath", profile], capture_output=True, text=True, timeout=10).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return None
    folder = Path(home) / ".claude" / "projects"
    return folder if home and folder.is_dir() else None


def link_windows_projects():
    folder = windows_projects()
    if not folder:
        return
    PROJECTS.mkdir(parents=True, exist_ok=True)
    linked = 0
    for win_dir in folder.iterdir():
        match = re.fullmatch(r"([A-Za-z])--(.*)", win_dir.name)
        target = PROJECTS / f"-mnt-{match[1].lower()}-{match[2]}" if match else None
        # A real WSL folder or a link made by hand is never replaced.
        if target and win_dir.is_dir() and not target.exists() and not target.is_symlink():
            try:
                target.symlink_to(win_dir)
                linked += 1
            except OSError as e:
                print(f"could not link {win_dir.name}: {e}", file=sys.stderr)
    if linked:
        print(f"linked {linked} new Windows project(s)")


def on_disk_case(path):
    # /mnt/c is case-insensitive, so `cd Pixlr` works for a folder named pixlr, but Claude keys history by the
    # typed spelling. Rebuild the path with the casing actually on disk.
    real = Path("/")
    for part in Path(path).parts[1:]:
        try:
            names = os.listdir(real)
        except OSError:
            names = []
        real /= part if part in names else next((n for n in names if n.lower() == part.lower()), part)
    return str(real)


def as_wsl_path(path):
    match = re.fullmatch(r"([A-Za-z]):[\\/](.*)", path) or re.fullmatch(r"/([A-Za-z])/(.*)", path)
    if match and not os.path.isdir(path):
        return f"/mnt/{match[1].lower()}/{match[2].replace(chr(92), '/')}"
    return path


def describe(path):
    """(title, folder the session was started in), or None for a session with nothing to show."""
    first_prompt, custom, generated, cwd = None, None, None, None
    with open(path, "rb") as lines:
        for line in lines:
            # Session files run to tens of MB; parse only lines that can hold a title or the first prompt.
            if not (b'"custom-title"' in line or b'"ai-title"' in line
                    or (cwd is None and b'"cwd"' in line) or (first_prompt is None and b'"type":"user"' in line)):
                continue
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            cwd = cwd or entry.get("cwd")
            kind = entry.get("type")
            if kind == "custom-title" and entry.get("customTitle"):
                custom = entry["customTitle"]
            elif kind == "ai-title" and entry.get("aiTitle"):
                generated = entry["aiTitle"]
            elif kind == "user" and first_prompt is None and not entry.get("isMeta"):
                content = entry.get("message", {}).get("content")
                if isinstance(content, list):
                    content = " ".join(b.get("text", "") for b in content if isinstance(b, dict))
                if isinstance(content, str) and content.strip() and not content.lstrip().startswith("<"):
                    first_prompt = content
    text = custom or generated or first_prompt
    return (" ".join(text.split()), as_wsl_path(cwd) if cwd else None) if text else None


def load_cache():
    try:
        return json.loads(CACHE.read_text())
    except (OSError, ValueError):
        return {}


def describe_cached(path, cache):
    """describe(), remembered until the file changes. Most of the cost is reading over /mnt/c."""
    stat = path.stat()
    key, stamp = str(path), [stat.st_size, stat.st_mtime_ns]
    hit = cache.get(key)
    if hit and hit[0] == stamp:
        return tuple(hit[1]) if hit[1] else None
    found = describe(path)
    cache[key] = [stamp, list(found) if found else None]
    return found


def save_cache(cache):
    try:
        CACHE.parent.mkdir(parents=True, exist_ok=True)
        CACHE.write_text(json.dumps(cache))
    except OSError:
        pass


def main():
    args = sys.argv[1:]
    extra = args[args.index("--") + 1:] if "--" in args else []
    args = args[: args.index("--")] if "--" in args else args
    every_project = "-a" in args
    if {"-h", "--help"} & set(args):
        print(__doc__.strip())
        return
    args = [a for a in args if a != "-a"]
    if len(args) > 1 or (args and not args[0].isdigit()):
        sys.exit(__doc__.strip())
    count = int(args[0]) if args else 20

    link_windows_projects()
    cwd = on_disk_case(os.getcwd())
    folders = [p for p in PROJECTS.iterdir() if p.is_dir()] if every_project else [PROJECTS / project_key(cwd)]
    files = [f for folder in folders if folder.is_dir() for f in folder.glob("*.jsonl")]
    files.sort(key=lambda p: p.stat().st_mtime, reverse=True)

    cache = load_cache()
    sessions = []
    for path in files:
        if len(sessions) == count:
            break
        if found := describe_cached(path, cache):
            sessions.append((path, *found))
    save_cache(cache)
    if not sessions:
        sys.exit(f"no sessions for {cwd}" + ("" if every_project else " (try: cresume -a)"))

    width = max(shutil.get_terminal_size().columns - (36 if every_project else 20), 20)
    for i, (path, text, started_in) in enumerate(sessions, 1):
        when = time.strftime("%b %d %H:%M", time.localtime(path.stat().st_mtime))
        project = f"{Path(started_in or '?').name[:14]:<15} " if every_project else ""
        print(f"{i:>3}  {when}  {project}{text[:width]}")
    try:
        choice = input("resume which (Enter to cancel)? ").strip()
    except EOFError:
        return
    if not choice:
        return
    if not choice.isdigit() or not 1 <= int(choice) <= len(sessions):
        sys.exit("not a number from the list")
    path, _, started_in = sessions[int(choice) - 1]

    # --resume <id> looks the session up under the current folder's history, so run from the folder it
    # belongs to.
    run_in = cwd
    if every_project and started_in and os.path.isdir(started_in):
        run_in = on_disk_case(started_in)
    if project_key(run_in) != path.parent.name:
        sys.exit(f"cannot tell which folder session {path.stem} belongs to; cd there and run cresume")
    os.chdir(run_in)
    claude = shutil.which("claude") or str(Path.home() / ".local" / "bin" / "claude")
    os.execv(claude, [claude, "--resume", path.stem, *extra])


if __name__ == "__main__":
    main()
