---
name: reflect
description: After a porting session, turn what was learned into durable rules and journal entries so the next run is faster. Runs in a fork; never loosens a gate.
context: fork
disallowed-tools: AskUserQuestion
---

# Reflect

Turn this session's experience into files on disk. Context evaporates at compaction;
files do not. A loop that does not write back is a recurring task, not a self-improving
one.

State: !`git log --oneline -10`
Journal size: !`wc -l migration/JOURNAL.md 2>/dev/null || echo "0 (no journal yet)"`
Blocked: !`ls migration/blocked/ 2>/dev/null | tr '\n' ' '`
Shims: !`make shim-report 2>/dev/null | head -4`

## What you may change

- `migration/JOURNAL.md` — append. Format decisions discovered, offsets that turned
  out to mean something specific, dead ends and why they were dead.
- `.claude/rules/*.md` — add or sharpen a rule when a mistake would plausibly recur.
- `migration/blocked/*.md` — add diagnosis to an existing entry, or remove one that a
  later unit unblocked.
- `.claude/skills/port-unit/SKILL.md` — only the "What you see / what it means"
  divergence table, and only to add a row you empirically confirmed.

## What you may never change

Anything that defines "done": `harness/`, `Makefile`, `upstream/`, `CLAUDE.md`,
`.claude/settings.json`, `.claude/hooks/`. A PreToolUse hook blocks these, and the
block is not a bug to route around.

This restriction is the point of the skill. An unsupervised improvement loop optimises
for whatever makes it stop sooner, and the shortest path to "done" is always a weaker
definition of done. If you believe a gate is genuinely wrong, append a **Proposals for
the human** section to `migration/JOURNAL.md` with the evidence. Do not act on it.

## Method

1. **Read the last ~5 journal entries** before writing anything. You are looking for
   whether what you just saw is new or a repeat.

2. **Weigh by recurrence, not recency.** One occurrence is an observation — record it
   in the journal and stop. Two occurrences across separate sessions is a pattern —
   now write the rule. A rules file that thrashes on single bad runs is worse than no
   rules file, because it teaches the next run to distrust it.

3. **Downweight rather than delete.** When a prior rule turned out to be wrong in one
   case, add the exception with its condition. Deleting it loses the case where it was
   right.

4. **Check convergence.** Compare the shim/boundary count against the last two
   reflections recorded in the journal. Rising twice running means the port is
   spreading rather than converging: say so explicitly, and recommend consolidating
   existing units before starting new ones.

5. **Audit the blocked directory.** For each entry, ask whether a unit ported since
   then unblocks it. If so, note that in the file so the next `/port-unit` picks it up.

   An empty `migration/blocked/` after a week of autonomous running is not good news.
   A loop that never parks anything is either not attempting anything hard or is
   resolving ambiguity by guessing. Say so if you see it.

6. **Write a dated entry** with: units attempted, units landed, what `make verify`
   actually reported, the boundary count, and one sentence on what would most speed up
   the next session.

## Output

Report what you wrote and what you deliberately did not write. If you observed
something that looks like a gate problem, quote it and say you have journalled it as a
proposal rather than acting on it.
