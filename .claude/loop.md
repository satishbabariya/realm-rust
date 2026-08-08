Continue the realm-core → Rust port. One unit per iteration.

1. If `migration/BOOTSTRAP-REPORT.md` does not exist or ends in NO-GO, stop and say so.
   Do not port anything against an unverified harness.

2. Pick the first unit in `migration/queue.md` that is neither already ported nor
   present in `migration/blocked/`. Run `/port-unit` on it.

3. If `make verify` exits 0, commit and continue to the next unit.
   If it does not, either fix it or park the unit in `migration/blocked/` with a real
   diagnosis — offsets you saw, hypotheses you tested, what you would need to proceed.
   Parking is a successful iteration. Grinding on the same offset for a third
   hypothesis is not.

4. Every fifth iteration, run `/reflect` instead of porting.

Stop the loop and report if any of these happens:

- `make determinism-check` starts failing. The oracle disagreeing with itself
  invalidates every green result since the last time it passed, so nothing after that
  point is trustworthy.
- Three consecutive units end up in `migration/blocked/`. The queue order is probably
  wrong and continuing just fills the directory.
- The boundary count in `make shim-report` rises for two reflections running. The port
  is spreading rather than converging; that is a design problem, not a throughput one.
- Any gate would need to change for a unit to pass. Never negotiate with the gate —
  park and stop.

Never edit `upstream/`, `harness/`, the `Makefile`, or anything under `.claude/` that
constrains this loop.
