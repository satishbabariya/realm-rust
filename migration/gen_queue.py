#!/usr/bin/env python3
"""Generate migration/queue.md: realm-core translation units, ordered by what the gate
can actually judge.

Ordering rationale, rewritten 2026-08-09
----------------------------------------
The original ordering was (depth, lines). Neither predicts whether a unit is portable
or whether `make verify` can tell a correct port from an empty one, and over one
session it sent the loop somewhere worse than an available alternative seven times:

    util/compression #14   947 lines   7 vtables, real throws, and its payload never links
    util/demangle    #18    48 lines   throws; the one function it exists for never links
    version          #19    77 lines   0 of 6 realm symbols link
    array_with_find  #22    83 lines   one unit-private vtable
    util/resource_limits #23 122 lines 0 of 4 symbols link
    util/platform_info #26 145 lines   0 of 2 symbols link
    array_timestamp  #32   264 lines   live and portable, but NO trace reaches it

Three of those are unreachable, which one `nm` intersection detects in about a second.
The last one is the sharper case: it is live and portable, and the gate still cannot
judge it, because no trace exercises a Timestamp.

So the queue now carries two measured columns and sorts on them.

  linked  realm-owned symbols this unit defines that survive into build/oracle/trace_runner.
          0 means `evidence-and-linkage.md` category 1: porting it would produce a green
          `make verify` that substantiates nothing. Those sort to the bottom.

  status  ported / blocked / pending, read from crates/realm-core-rs/ported_units.txt and
          migration/blocked/. Units already decided sort to the bottom so the head of the
          queue is always live work.

Ordering among pending, reachable units is (depth, -inbound, lines): shallow first
because a unit whose dependencies are still C++ needs a shim; high inbound next because
it unblocks more; size only as a tiebreak, since size measures effort and not value.

`linked` needs the oracle built. Without it the column reads `?` and the ordering falls
back to the old (depth, lines) — the file says so, so a queue generated before
`make oracle` is never mistaken for a measured one.

There is a third column worth having and not computed here: whether any trace reaches
the unit. Measuring it needs an lldb breakpoint sweep per unit per trace, which is
minutes per unit and cannot run over 100 units in a generator. Measure it for the head
of the queue by hand — the recipe is in JOURNAL.md under the array_blobs entry — and
prefer a traced unit over an untraced one when both are otherwise clean.

Run from the repo root: python3 migration/gen_queue.py
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "upstream" / "src" / "realm"
OBJDIR = ROOT / "build" / "oracle" / "realm-core" / "src" / "realm" / "CMakeFiles" / "Storage.dir"
ORACLE = ROOT / "build" / "oracle" / "trace_runner"
PORTED = ROOT / "crates" / "realm-core-rs" / "ported_units.txt"
BLOCKED = ROOT / "migration" / "blocked"

INCLUDE = re.compile(rb'^\s*#\s*include\s*[<"]([^">]+)[">]', re.M)
# Symbols this project counts as "realm's own". Anything else a unit defines is a weak
# libc++ instantiation that links regardless and says nothing about the unit -- the
# mistake reflection #5 corrected.
REALM_SYM = re.compile(r"^__ZN[A-Z]*5realm")

SKIP = ("object-store/", "sync/", "parser/", "exec/", "tools/", "metrics/")


def rel(p: Path) -> str:
    return str(p.relative_to(ROOT))


def nm(args: list[str]) -> list[str]:
    try:
        out = subprocess.run(["nm", *args], capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.SubprocessError):
        return []
    if out.returncode != 0:
        return []
    return out.stdout.splitlines()


def linked_symbols() -> set[str] | None:
    """Every symbol name present in the linked oracle binary."""
    if not ORACLE.exists():
        return None
    names = set()
    for line in nm([str(ORACLE)]):
        parts = line.split()
        if parts:
            names.add(parts[-1])
    return names or None


def defined_realm_symbols(key: str) -> set[str] | None:
    obj = OBJDIR / f"{key}.cpp.o"
    if not obj.exists():
        return None
    names = set()
    for line in nm(["-g", str(obj)]):
        if " U " in line:
            continue
        parts = line.split()
        if parts and REALM_SYM.match(parts[-1]):
            names.add(parts[-1])
    return names


def load_ported() -> set[str]:
    if not PORTED.exists():
        return set()
    out = set()
    for line in PORTED.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.add(re.sub(r"\.cpp$", "", line))
    return out


def load_blocked() -> set[str]:
    if not BLOCKED.is_dir():
        return set()
    # blocked files name the unit with '/' replaced by '-'
    return {p.stem.replace("-", "/", 1) if "/" not in p.stem else p.stem
            for p in BLOCKED.glob("*.md")}


def blocked_matches(key: str, blocked_stems: set[str]) -> bool:
    return key.replace("/", "-") in {s.replace("/", "-") for s in blocked_stems}


def assign_depths(units: dict[str, dict]) -> None:
    """Set units[k]["depth"] = longest include chain within upstream/src/realm/.

    Rewritten 2026-08-09 (the third time this file's depth column has been wrong, and the
    first time it was *measured* wrong rather than argued wrong).

    The previous implementation memoised a depth that had been computed under a specific
    `seen` set:

        def depth(k, seen):
            if k in memo: return memo[k]
            if k in seen or k not in units: return 0
            d = 1 + max(depth(x, seen | {k}) for x in deps)
            memo[k] = d

    Realm's include graph has cycles, so `seen` is load-bearing: it is what cuts the
    cycle. Caching the result globally reuses a value that is only valid for the path it
    was reached by. Combined with `deps` being a `set` — whose iteration order varies per
    process with PYTHONHASHSEED — the whole column was nondeterministic. Four consecutive
    runs over an unchanged tree gave `array_integer` depths of 1, 30, 34 and 24, and the
    queue's top 50 rows reshuffled by ~55 lines each time.

    That is worse than a wrong number: the queue's stated purpose is to rank by what the
    gate can judge, and the primary sort key was noise. Any "ported out of queue order"
    note in JOURNAL.md written before this date compares against an ordering that would
    not reproduce.

    "Longest chain" is undefined on a cycle, so the fix is to make the question
    well-formed rather than to pick a better cut: condense strongly-connected components
    (Tarjan) and take the longest path over the resulting DAG. Every unit in a cycle gets
    the same depth, which is the honest answer — mutually-including headers have no
    ordering between them. Depth 0 still means "pulls in nothing else from realm", and
    that is now a property of the graph rather than of the walk order.

    Deterministic by construction: no memo across paths, and every iteration over `deps`
    is sorted.
    """
    keys = sorted(units)
    adj = {k: sorted(d for d in units[k]["deps"] if d in units) for k in keys}

    # --- Tarjan SCC, iterative: realm's include graph is deep enough to blow the
    # recursion limit, and a RecursionError here would look like a missing unit.
    index: dict[str, int] = {}
    low: dict[str, int] = {}
    on_stack: set[str] = set()
    stack: list[str] = []
    comp_of: dict[str, int] = {}
    comps: list[list[str]] = []
    counter = 0

    for root in keys:
        if root in index:
            continue
        work = [(root, 0)]
        while work:
            v, pi = work[-1]
            if pi == 0:
                index[v] = low[v] = counter
                counter += 1
                stack.append(v)
                on_stack.add(v)
            if pi < len(adj[v]):
                work[-1] = (v, pi + 1)
                w = adj[v][pi]
                if w not in index:
                    work.append((w, 0))
                elif w in on_stack:
                    low[v] = min(low[v], index[w])
            else:
                if low[v] == index[v]:
                    comp: list[str] = []
                    while True:
                        w = stack.pop()
                        on_stack.discard(w)
                        comp_of[w] = len(comps)
                        comp.append(w)
                        if w == v:
                            break
                    comps.append(comp)
                work.pop()
                if work:
                    low[work[-1][0]] = min(low[work[-1][0]], low[v])

    # --- longest path over the condensation. Tarjan emits components in reverse
    # topological order, so a single forward pass suffices: every successor component
    # has a lower index and is already final.
    comp_depth = [0] * len(comps)
    for ci, comp in enumerate(comps):
        best = -1
        for v in comp:
            for w in adj[v]:
                cj = comp_of[w]
                if cj != ci:
                    best = max(best, comp_depth[cj])
        # A cycle costs one level for the whole group, not one per member.
        comp_depth[ci] = best + 1

    for k in keys:
        units[k]["depth"] = comp_depth[comp_of[k]]
        # How many units share this depth *because they mutually include each other*.
        # 1 means the depth is a real ordering statement about this unit; >1 means the
        # unit sits in a cycle and depth cannot rank it against its cycle-mates.
        units[k]["scc_size"] = len(comps[comp_of[k]])


def main() -> int:
    if not SRC.is_dir():
        print("upstream/src/realm not found — run: git submodule update --init --recursive",
              file=sys.stderr)
        return 2

    sources = [p for p in SRC.rglob("*.cpp")
               if not any(s in str(p.relative_to(SRC)) for s in SKIP)]

    units: dict[str, dict] = {}
    for cpp in sources:
        key = str(cpp.relative_to(SRC).with_suffix(""))
        hpp = cpp.with_suffix(".hpp")
        text = cpp.read_bytes() + (hpp.read_bytes() if hpp.exists() else b"")
        deps = set()
        for inc in INCLUDE.findall(text):
            i = inc.decode("utf8", "replace")
            if i.startswith("realm/"):
                d = i[len("realm/"):]
                d = re.sub(r"\.(hpp|h)$", "", d)
                if d != key:
                    deps.add(d)
        units[key] = {
            "cpp": rel(cpp),
            "lines": len(cpp.read_bytes().splitlines()),
            "deps": deps,
        }

    assign_depths(units)

    inbound = {k: 0 for k in units}
    for k, u in units.items():
        for d in u["deps"]:
            if d in inbound:
                inbound[d] += 1

    linked = linked_symbols()
    measured = linked is not None
    ported = load_ported()
    blocked_stems = load_blocked()

    for k, u in units.items():
        if measured:
            defined = defined_realm_symbols(k)
            if defined is None:
                u["linked"] = None       # object not built (e.g. excluded from this target)
            else:
                u["linked"] = len(defined & linked)
                u["defined"] = len(defined)
        else:
            u["linked"] = None
        if k in ported:
            u["status"] = "ported"
        elif blocked_matches(k, blocked_stems):
            u["status"] = "blocked"
        else:
            u["status"] = "pending"

    STATUS_RANK = {"pending": 0, "blocked": 1, "ported": 2}

    def sort_key(kv):
        k, u = kv
        # Unreachable units sort behind reachable ones: a green verify on them proves
        # nothing, so they are never the right next unit.
        unreachable = 1 if (u["linked"] == 0) else 0
        return (STATUS_RANK[u["status"]], unreachable, u["depth"], -inbound[k], u["lines"], k)

    order = sorted(units.items(), key=sort_key)
    total = len(order)
    pending = [kv for kv in order if kv[1]["status"] == "pending"]
    shown = order[:60]

    def cell(v) -> str:
        return "?" if v is None else str(v)

    big_scc = max(u["scc_size"] for u in units.values())

    out = [
        "# Port queue",
        "",
        f"Generated by `migration/gen_queue.py` from realm-core at "
        f"`upstream/src/realm/`. {total} translation units total; the first 60 are listed.",
        "",
        "**Ordering is by what the gate can judge, not by size.** Pending units first, then",
        "reachable before unreachable, then by dependency depth, then by `inbound`",
        "descending, and only then by size. Size measures effort, not value; ranking on it",
        "sent the loop at dead or ungradeable code seven times in one session.",
        "",
        "| column | meaning |",
        "|---|---|",
        "| `depth` | longest chain of `#include`s within `upstream/src/realm/`, over the DAG of "
        "strongly-connected components. 0 = pulls in nothing else from realm |",
        "| `inbound` | how many other units depend on this one. High inbound at low depth unblocks the most |",
        "| `lines` | size of the `.cpp`. A tiebreak, nothing more |",
        "| `linked` | realm-owned symbols the unit defines that survive into `build/oracle/trace_runner` |",
        "| `status` | `pending`, `blocked` (has a file in `migration/blocked/`), or `ported` |",
        "",
        "**`linked` = 0 means park it.** Category 1 of `.claude/rules/evidence-and-linkage.md`:",
        "the linker never pulls the unit, so a green `make verify` on it substantiates nothing.",
        "A *partial* count is not reassurance either — check whether the linked subset contains",
        "the functions the unit is named for (`util/demangle` linked 5 of 6 and the missing one",
        "was its entire payload).",
        "",
        f"**`depth` cannot rank the core.** {big_scc} of {len(units)} units form a single",
        "strongly-connected component — `alloc`, `array`, `table`, `group` and `db` all",
        "mutually `#include` each other — so they share one depth and depth says nothing about",
        "their relative order. Among those, `inbound` and the screen in",
        "`.claude/rules/unit-screening.md` are the only ranking signals; a low depth here means",
        "\"outside the core cycle\", not \"early in a chain\".",
        "",
        "Not computed here: whether any **trace** reaches the unit. That needs an lldb",
        "breakpoint sweep per unit per trace — minutes each, so it cannot run over 100 units",
        "in a generator. Measure it by hand for the head of the queue and prefer a traced unit",
        "over an untraced one when both are clean; `array_timestamp` is live, portable and",
        "reached by no trace, which makes it a worse next unit than it looks here.",
        "",
        "Do not hand-edit this file — regenerate it. Record progress in JOURNAL.md.",
        "",
    ]
    if not measured:
        out += [
            "> **`linked` is unmeasured** — `build/oracle/trace_runner` was not present when this",
            "> was generated, so the column reads `?` and the ordering fell back to depth-then-size.",
            "> Run `make oracle` and regenerate before trusting the order.",
            "",
        ]
    out += [
        "| # | unit | depth | inbound | lines | linked | status |",
        "|---|---|---|---|---|---|---|",
    ]
    for i, (k, u) in enumerate(shown, 1):
        out.append(
            f"| {i} | `{u['cpp']}` | {u['depth']} | {inbound[k]} | {u['lines']} | "
            f"{cell(u['linked'])} | {u['status']} |"
        )

    d0 = sum(1 for _, u in order if u["depth"] == 0)
    unreachable = sum(1 for _, u in order if u["linked"] == 0)
    out += [
        "",
        f"Depth-0 (shim-free) units: **{d0}** of {total}. "
        f"Pending: **{len(pending)}**. "
        + (f"Unreachable (`linked` = 0): **{unreachable}**." if measured else ""),
        "",
        "If the depth-0 number is small, the port will be shim-heavy from the start and the",
        "boundary count in `make shim-report` will rise before it falls. That is expected;",
        "what is not expected is it continuing to rise across two reflection passes.",
    ]
    (ROOT / "migration" / "queue.md").write_text("\n".join(out) + "\n")
    print(f"wrote migration/queue.md — {total} units, {len(pending)} pending, "
          f"{d0} at depth 0, {unreachable if measured else '?'} unreachable")
    return 0


if __name__ == "__main__":
    sys.exit(main())
