#!/usr/bin/env bash
# PreToolUse guard: refuse writes to anything that defines "done".
#
# This exists because a permission rule can be reasoned around and a hook cannot.
# An agent stuck on a failing diff-test will eventually notice that editing the
# trace, the comparator, or the oracle makes the failure go away. That is the one
# failure mode this project cannot tolerate, so it is blocked deterministically.
#
# Covers Write/Edit/NotebookEdit (via tool_input.file_path) AND Bash (via a scan of
# the command string). The Bash arm is deliberately coarse: it is a tripwire, not a
# sandbox. A determined shell one-liner can still evade it, which is why the file
# list below is short enough for a human to eyeball in a diff.
#
# Exit 2 blocks the call and returns stderr to the model as feedback.

set -uo pipefail

payload=$(cat)

read -r tool_name path command < <(printf '%s' "$payload" | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("? ? ?"); sys.exit(0)
ti = d.get("tool_input") or {}
name = d.get("tool_name") or "?"
path = (ti.get("file_path") or ti.get("path") or "?").replace(" ", "\\ ")
cmd  = (ti.get("command") or "?").replace("\n", " ")
print(name, path, cmd)
' 2>/dev/null)

root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

# Bootstrap escape. The guard protects the steady state, but the harness itself has
# to be built by someone, and during construction every write it protects is
# legitimate. A human creates this sentinel to build or repair the harness and
# deletes it before starting the autonomous loop.
#
# .claude/settings.json denies Write/Edit to this path, and this script refuses to
# honour a sentinel it cannot attribute to a human. Neither is airtight -- see the
# header. It is a tripwire whose value is that evading it leaves a trace in the
# transcript that a human reading the diff will notice.
if [ -f "$root/.claude/BOOTSTRAP-UNLOCKED" ]; then
  echo "guard-baseline: BOOTSTRAP-UNLOCKED present — baseline writes permitted." >&2
  exit 0
fi

deny() {
  cat >&2 <<EOF
BLOCKED: $1

$2

This is not a permission you can request differently — the target defines what
"done" means, and a run that edits it can no longer demonstrate anything.

If you believe the baseline itself is wrong:
  1. Stop working on the current unit.
  2. Write migration/blocked/<unit>.md explaining precisely what is wrong,
     what you observed, and what you would change.
  3. Move to the next unit in migration/queue.md.

A human resolves it. That is the whole protocol.
EOF
  exit 2
}

reason_for() {
  case "$1" in
    upstream/*)
      echo "upstream/ is the oracle. If it changes, there is no reference to compare against and every prior green result becomes unverifiable." ;;
    harness/traces/*)
      echo "Traces are the specification. Making a trace weaker is indistinguishable, in the log, from making the port correct." ;;
    harness/*)
      echo "The harness is the measuring instrument. Changing it mid-run invalidates the comparison it exists to make." ;;
    Makefile)
      echo "The Makefile defines the gate commands. Loosening a gate is the fastest possible route to green and the least informative one." ;;
    migration/queue.md)
      echo "The queue is the plan of record. Record progress in migration/JOURNAL.md instead." ;;
    CLAUDE.md|.claude/settings.json|.claude/settings.json.*|.claude/hooks/*)
      echo "This file constrains the loop. A loop that can edit its own constraints has none." ;;
    *) echo "" ;;
  esac
}

# --- file-based tools -------------------------------------------------------
if [ "$path" != "?" ] && [ -n "$path" ]; then
  case "$path" in
    /*) rel="${path#$root/}" ;;
    *)  rel="$path" ;;
  esac
  reason=$(reason_for "$rel")
  [ -n "$reason" ] && deny "$rel" "$reason"
fi

# --- Bash ------------------------------------------------------------------
# Only inspect commands that could plausibly write. Read-only use of these paths
# (grep, cat, nm, make) must stay unimpeded or the loop cannot do its job.
if [ "$tool_name" = "Bash" ] && [ "$command" != "?" ]; then
  case "$command" in
    *">"*|*"tee "*|*"sed -i"*|*"cp "*|*"mv "*|*"rm "*|*"truncate"*|*"patch "*|*"git checkout"*|*"git restore"*|*"git apply"*|*"dd "*)
      for guarded in "upstream/" "harness/" "Makefile" "migration/queue.md" "CLAUDE.md" ".claude/settings.json" ".claude/hooks/"; do
        case "$command" in
          *"$guarded"*)
            reason=$(reason_for "${guarded%/}")
            [ -z "$reason" ] && reason=$(reason_for "$guarded")
            deny "Bash command touching $guarded" "${reason:-This path defines the acceptance criterion.}
Command was:
  $command

If this was a read-only use of that path, rewrite it without a redirect or a
copy/move/remove so the intent is unambiguous."
            ;;
        esac
      done
      ;;
  esac
fi

exit 0
