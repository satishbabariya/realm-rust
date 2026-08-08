---
name: bootstrap
description: Phase 0 for the realm-core port — verify toolchain, build the oracle, prove the format is deterministic, seed the corpus, generate the port queue, and emit a GO/NO-GO report. Run once before any porting.
disallowed-tools: AskUserQuestion
---

# Bootstrap the migration harness

You are establishing whether byte-identity is a usable acceptance criterion for
porting realm-core to Rust. Nothing gets ported until that question is answered.

You may not ask the user anything. Where a decision is needed, make it, and record
it under "Decisions" in the report.

## Current state

Repo: !`pwd`
Upstream pin: !`git -C upstream describe --tags 2>/dev/null || echo "MISSING — submodule not initialised"`
Traces: !`ls harness/traces/*.trace 2>/dev/null | wc -l | tr -d ' '`
Oracle built: !`test -x build/oracle/trace_runner && echo yes || echo no`
Corpus: !`ls migration/corpus/*.realm 2>/dev/null | wc -l | tr -d ' '` files

## Steps

Work through these in order. Do not skip ahead on the assumption that a later step
will catch an earlier failure — steps 6 and 7 are the ones that can invalidate the
whole project, and they are only meaningful if 1–5 actually succeeded.

1. **Submodules.** `git submodule update --init --recursive`. realm-core needs
   `src/external/sha-1`, `src/external/sha-2`, and `test/external/catch`.

2. **`make doctor`.** Must exit 0. If a tool is missing, install it if you can do so
   non-interactively; otherwise record it as a NO-GO blocker and continue collecting
   the other blockers rather than stopping at the first one.

3. **`make oracle`.** Cold build is 10–20 minutes. Run it in the background and poll
   its output file rather than blocking. Watch for `error:` — warnings are expected
   and fine.

   If the build fails inside `upstream/`, do **not** edit upstream. Fix it in
   `harness/CMakeLists.txt` with a compile flag, and note it in the report. A known
   example is already handled there: clang ≥ 21 rejects s2's `std::is_pod`
   specialization, suppressed with `-Wno-invalid-specialization`.

4. **`make hybrid`.** With no Rust crate yet this is a second build of the same
   sources, and CMake will warn that the hybrid is identical to the oracle by
   construction. That warning is correct right now and a bug at any point after the
   first unit lands.

5. **`make corpus`.** Seeds `migration/corpus/` from upstream's legacy `.realm`
   fixtures. Files with `decrypt` in the name are excluded on purpose.

6. **`make determinism-check`.** **This is the load-bearing step.** It runs every
   trace through the oracle twice and byte-compares the oracle against itself.

   If it fails, stop. Do not proceed to step 7, do not port anything, and write the
   report with **NO-GO**. A non-deterministic oracle means byte-identity cannot be
   the acceptance criterion, and the correct next move is a human decision about what
   to compare instead — not a workaround you invent. Include the exact offset and
   region `compare_realm.py` reported; that usually identifies what run-varying state
   is leaking into the file.

7. **`make diff-test`.** Right now the hybrid *is* the oracle, so every trace must
   pass trivially. If any trace fails here, the harness is broken — the two builds
   differ in some way you have not accounted for — and nothing downstream can be
   trusted. This is a NO-GO.

8. **`make format-compat`.** Both stacks open every corpus file and must agree on the
   schema fingerprint and produce byte-identical upgraded files. Some fixtures will be
   rejected by both stacks; that is a `skip`, not a failure — agreement is the check.

9. **Generate `migration/queue.md`.** Enumerate the translation units under
   `upstream/src/realm/` and order them by dependency depth, leaves first. Use
   `#include` edges within `upstream/src/realm/` to compute depth; treat standard
   library and external includes as depth 0. For each unit record: path, line count,
   inbound dependency count, and the depth. The first units to port must be pure leaf
   utilities with no realm-internal includes — `realm/util/*` is where to look.

   Cap the file at the first 60 units with a note of the total; a 400-line queue is a
   worse planning artifact than a 60-line one.

10. **Write `migration/BOOTSTRAP-REPORT.md`.** Sections: Environment, What was built,
    Determinism result (with evidence — the actual command output, not a summary),
    Format-compat result, Queue summary, Decisions I made, Blockers. End the file with
    a line that is exactly `GO` or `NO-GO` and nothing else.

## What GO means

`GO` means all of: doctor 0, both stacks built, determinism-check 0, diff-test 0,
format-compat 0, queue generated. Anything less is `NO-GO` with the specific reason.

Do not report GO on the basis that a command *should* pass. Every claim in the report
must cite output you actually saw in this session.

## After the report

State plainly whether it is GO or NO-GO and what the single next action is. Do not
start porting — bootstrap ends here.

Recommend, in your closing message, that the user do one supervised sanity check
before starting the autonomous loop: port one trivial unit, then deliberately break
it (widen an integer, flip a byte order) and confirm `make diff-test` actually fails.
A harness that has never failed on this machine proves nothing when it passes.
