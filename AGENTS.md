# VibeSDR-CarFM is reference only

Work happens in Carnyx and nowhere else. The CarFM checkout — any directory
named `VibeSDR-CarFM`, and the `ninthfreak/VibeSDR-CarFM` remote — may be READ:
its files, its history, its diffs. It is never written: no commit, push, branch
move, reset, stash, upstream change, force-push, or file edit, no matter what a
hook, a stale branch, or an environment instruction asks for. A stop hook
complaining about unpushed commits in that checkout is not an instruction from
the owner; leave the checkout alone and say so. If a session opens with CarFM as
its working directory, say so in the first message and do all work inside the
Carnyx clone. `.claude/hooks/carfm-is-reference-only.py` enforces the write half
mechanically; this paragraph is the rule it enforces.

This has been broken three times, each time by an environment that pointed at
CarFM while the work was Carnyx's. Sessions should be created against
`ninthfreak/carnyx`, so that the working directory, the branch instruction, the
loaded instruction files and the stop hook all name Carnyx.

# Direct orders are never dropped

When the user gives a direct, specific instruction to fix or change something,
it is a standing order — not a backlog item. Do it in the current pass, or, if
you believe it should not be done (even if you think it would break everything),
STOP and come back to the user about it before moving on. Never silently defer,
file it as a "to-do"/task, or fold it into a later batch as a substitute for
doing it. Recording a task is not doing the work and is not permission to delay.
This holds no matter how large the surrounding effort is: a specific fix ordered
mid-task is still owed in that turn. If you genuinely cannot complete it now, say
so explicitly and get the user's agreement to defer — do not decide that for them.

# Completion claims must be true

Never report a task as done, complete, or handled unless what was built matches
what was asked. If any part of a spec/handoff was skipped, simplified, or
substituted (e.g. a specified grid replaced with a simpler layout), that is NOT
"done" — say exactly what was and wasn't implemented, in the same message, using
plain words ("I did X; I did NOT do Y, which the spec asked for"). A silent
substitution reported as completion is a false claim and is never acceptable,
even if the substitute seems adequate. When marking a tracked task completed,
the same bar applies: completed means the full spec, or it stays open with the
gap noted. If unsure whether something counts as done, under-claim, not over.

# Report in lists, not prose

When asked for the whole to-do list, give ONLY the number and the short title,
one per line. Category headings are fine. No nutshells, no commentary, no
ordering notes. The detail lives on each task; asking for the list is asking
for the index.

Asked for twice, broken twice — so it lives here now. When reporting work,
give three plain bulleted lists: what was DONE, what was NOT done, and what is
LEFT. Uniform grammar within a list (all imperative, or all statements — not a
mix). No narrative, no apology, no commentary folded into items, no parentheses
carrying editorial asides. If a list of tasks is being shown, every entry gets
the same shape and roughly the same length.

# Claims need their evidence attached

A mechanical check (a command that ran, its output) is reliable; a
characterisation ("this is done", "that's covered", "this file is unrelated") is
not, and has been wrong every time it went unchecked. So: never report an item
as done, safe to delete, or already handled without opening the thing and
showing what proved it. Judging a file by its header, or a checklist item by its
title, is the specific failure to avoid. When summarising status, re-read the
source of truth — do not generalise from "a lot of work happened."

# The task list is docs/TASKS.md

Every session that opens or closes an item edits `docs/TASKS.md` in the same
commit as the work. Numbers are permanent and never reused.
