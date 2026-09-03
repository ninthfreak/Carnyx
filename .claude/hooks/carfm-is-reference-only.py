#!/usr/bin/env python3
"""PreToolUse guard: VibeSDR-CarFM is reference only, and git writes stay in Carnyx.

Reads the hook's JSON from stdin. Denies:
  - any Write/Edit/NotebookEdit whose file_path is inside a VibeSDR-CarFM checkout;
  - any Bash command that writes into a VibeSDR-CarFM checkout: a git write verb
    with CarFM anywhere in the command, a redirection or copy/move whose target is
    in CarFM, or a file-mutating command given a CarFM path;
  - any Bash git write verb aimed outside this project via `cd`, `pushd`, `git -C`,
    `--git-dir` or `--work-tree`.
Reading CarFM (cat, grep, git log/diff/show, cp OUT of it) stays allowed.
"""
import json, os, re, shlex, sys

CARFM = re.compile(r"vibesdr[-_]?carfm", re.I)
GIT_WRITE = re.compile(
    r"\bgit\b(?:\s+-C\s+\S+|\s+--git-dir(?:=|\s+)\S+|\s+--work-tree(?:=|\s+)\S+|\s+-c\s+\S+)*\s+"
    r"(?:push|commit|merge|rebase|reset|checkout|switch|restore|stash|cherry-pick|revert|am|apply|"
    r"tag|clean|branch\s+(?:-[dDmMu]|--(?:delete|move|set-upstream-to|unset-upstream))|"
    r"remote\s+(?:add|remove|rm|set-url|rename)|update-ref|symbolic-ref|filter-branch|gc|prune|worktree)\b"
)
MUTATORS = {"rm", "rmdir", "mv", "touch", "mkdir", "chmod", "chown", "ln", "truncate", "tee", "rsync", "dd", "shred", "unlink"}
COPYLIKE = {"cp", "mv", "rsync", "install"}


def deny(reason: str) -> None:
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "AGENTS.md: " + reason,
        }
    }))
    sys.exit(0)


def project_dir() -> str:
    return os.path.realpath(os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd())


def inside_project(path: str, base: str) -> bool:
    p = os.path.realpath(os.path.expanduser(path if os.path.isabs(os.path.expanduser(path)) else os.path.join(base, path)))
    root = project_dir()
    return p == root or p.startswith(root + os.sep)


def check_bash(cmd: str, session_cwd: str) -> None:
    if GIT_WRITE.search(cmd) and CARFM.search(cmd):
        deny("git write aimed at VibeSDR-CarFM, which is reference only")
    # Split into simple commands on ; && || | and newlines, tracking `cd`.
    # The starting directory is the SESSION's cwd from the hook input, not the
    # project dir: a session opened against CarFM runs a bare `git push` there,
    # and that is the exact case this file exists for.
    parts = re.split(r"\s*(?:&&|\|\||;|\||\n)\s*", cmd)
    cwd = os.path.realpath(session_cwd) if session_cwd else project_dir()
    for part in parts:
        try:
            toks = shlex.split(part)
        except ValueError:
            toks = part.split()
        if not toks:
            continue
        # Directory changes carry forward.
        if toks[0] in ("cd", "pushd") and len(toks) > 1:
            target = os.path.expanduser(toks[1])
            cwd = target if os.path.isabs(target) else os.path.join(cwd, target)
            cwd = os.path.realpath(cwd)
            continue
        if GIT_WRITE.search(part):
            m = re.search(r"\bgit\s+-C\s+(\S+)", part) or re.search(r"--work-tree(?:=|\s+)(\S+)", part) or re.search(r"--git-dir(?:=|\s+)(\S+)", part)
            where = m.group(1) if m else cwd
            if not inside_project(where, cwd):
                deny(f"git write outside this project ({where}); Carnyx is the only repository this session writes to")
        # Redirections into CarFM.
        for m in re.finditer(r">{1,2}\s*(\S+)", part):
            if CARFM.search(m.group(1)):
                deny("redirecting output into VibeSDR-CarFM, which is reference only")
        # Copy-like commands: the LAST argument is the target.
        if toks[0] in COPYLIKE and len(toks) > 2 and CARFM.search(toks[-1]):
            deny(f"{toks[0]} into VibeSDR-CarFM, which is reference only")
        # In-place mutators given a CarFM path.
        if toks[0] in MUTATORS and any(CARFM.search(t) for t in toks[1:]):
            deny(f"{toks[0]} on a VibeSDR-CarFM path, which is reference only")
        if toks[0] in ("sed", "perl") and any(t.startswith("-i") for t in toks[1:]) and any(CARFM.search(t) for t in toks[1:]):
            deny("in-place edit of a VibeSDR-CarFM file, which is reference only")
        # A subshell or interpreter handed text that mentions CarFM together with
        # a write: python/bash -c with open(..., "w") etc. Coarse on purpose.
        if toks[0] in ("python", "python3", "bash", "sh", "node") and CARFM.search(part) and re.search(r"['\"]w['\"]|writeFile|write_text|open\([^)]*['\"][wa]", part):
            deny("a script writing into VibeSDR-CarFM, which is reference only")


def main() -> None:
    try:
        data = json.load(sys.stdin)
    except Exception:
        return
    tool = data.get("tool_name", "")
    inp = data.get("tool_input", {}) or {}
    if tool in ("Write", "Edit", "NotebookEdit", "MultiEdit"):
        path = str(inp.get("file_path") or inp.get("notebook_path") or "")
        if CARFM.search(path):
            deny("editing a file inside VibeSDR-CarFM, which is reference only")
        return
    if tool == "Bash":
        check_bash(str(inp.get("command") or ""), str(data.get("cwd") or ""))


if __name__ == "__main__":
    main()
