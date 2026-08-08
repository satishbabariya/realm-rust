# Running this

Phase 0 is done and verified on this machine. `make verify` exits 0, and — more
importantly — `make fault-check` proves the gate still bites.

## Before anything else: the ten-minute check

Do this once, by hand. It is the difference between a loop that ports realm-core and
one that produces confidently wrong units while every check stays green.

```bash
make fault-check
```

It builds a third stack with one written integer deliberately off by one and requires
`diff-test` to catch it. Current result on this machine: **5/5 traces caught it.**

If that ever comes back "THE GATE IS BLIND", stop. Nothing downstream means anything
until it is fixed.

## Phase 1 — one supervised unit

```bash
claude --permission-mode acceptEdits
```

```
/port-unit upstream/src/realm/util/base64.cpp
```

`migration/queue.md` lists 14 depth-0 (shim-free) units out of 103. `base64` is a good
first target: 209 lines, 5 inbound dependents, no realm-internal includes.

Watch for two things specifically:

- Does it run `make verify` and show you the output, or does it *claim* success? The
  Stop hook is configured to block the second case, and you want to see it work.
- Does it try to touch `harness/` or `upstream/` when stuck? The PreToolUse guard
  should block it. Seeing that block fire once is worth more than reading this file.

## Phase 2 — autonomous

```bash
claude --permission-mode auto
/loop 45m
```

`.claude/loop.md` drives it: pick the next unqueued unit, port it, verify, commit or
park; `/reflect` every fifth iteration.

### Stop it if

- `make determinism-check` starts failing — every green result since it last passed
  becomes unverifiable.
- Three consecutive units land in `migration/blocked/` — the queue order is wrong and
  continuing just fills the directory.
- The boundary count in `make shim-report` rises across two reflections — the port is
  spreading rather than converging.

### What to look at each morning

`migration/blocked/` first, not the commit count. A well-behaved loop parks what it
cannot honestly resolve. An **empty** blocked directory after a week is more likely to
mean the stop conditions are too weak than that everything went well.

Then `make shim-report`. The boundary count is the health metric; it should rise early
and then fall. The number of ported units is not a health metric.

## What is actually guaranteed here

Verified on this machine, with output I saw:

- realm-core v14.14.0 builds against Apple clang 21 (needs `-Wno-invalid-specialization`
  for s2's `std::is_pod` specialization — worked around in `harness/CMakeLists.txt`,
  not in `upstream/`).
- The format is **deterministic**: all 5 traces byte-identical across two oracle runs.
- The hybrid stack genuinely links the Rust staticlib — `make hybrid` greps the binary
  for a probe symbol and fails the build if ld dropped the archive. It caught a real
  arch mismatch during setup (Rosetta x86_64 shell, native arm64 rustc) that would
  otherwise have produced a green diff-test over pure C++.
- `format-compat`: 14 legacy fixtures open identically in both stacks, 9 rejected by
  both. 0 disagreements.
- The gate can fail: 5/5 on fault injection.

## What is not guaranteed

- **No unit has been ported.** `diff-test` passing right now proves the harness works,
  not that any Rust is correct.
- **Trace coverage is scalar-only.** The schema is one class with int/string/double/bool.
  Collections, links, Mixed, and Decimal128 are not exercised at all. The first unit
  that touches them needs a trace before it needs a port, and `/port-unit` will not
  notice the gap on its own — that one is on you.
- **The guard is a tripwire, not a sandbox.** It blocks Write/Edit to protected paths
  and scans Bash commands for redirects into them, but a determined shell one-liner
  can evade the Bash arm. Its value is that evading it leaves a trace in the transcript.
  Keep the protected file list short enough to eyeball in a diff.
- **`realm_compact()` runs before every close.** That removes allocator slack whose
  size varies with transient allocation order, which is what makes byte-comparison
  stable. It also means the traces never compare an *uncompacted* file, so a port that
  gets free-list layout wrong in a way compaction erases would not be caught. Worth a
  second trace family eventually.
