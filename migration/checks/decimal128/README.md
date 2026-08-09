# `decimal128.cpp` — ported

**Status: ported.** `make verify` exits 0 at 15 units. Two differentials cover it:
`run_conv_differential.sh` here for `realm_binary64_to_bid128` (15.2M comparisons), and
`migration/checks/run_decimal128_differential.sh` for the 47 method symbols (1,470 probe
lines).

Note what the gate does and does not say. No trace stores a decimal — the trace schema is
int/string/double/bool — so `make verify` proves the link is intact and that nothing else
regressed. The differentials are the evidence for the unit itself.

## Why this unit is shaped the way it is

`decimal128.cpp` is 1,848 lines, and the screen is the cleanest of any large unit left:
48 of 48 realm-owned symbols linked, **zero** `ZT*`, **zero** `VTT`, and one realm-undefined
symbol (`util::Printable::str() const`). `grep -cE '\b(throw|catch|try)\b'` is 0, and the
owning-function test finds only libc++ weak helpers (`__throw_length_error`,
`__throw_out_of_range`) and a `___clang_call_terminate` landing pad — the same clean shape
as `error_codes` and `status`.

But the line count hides where the work is:

| lines | what |
|---|---|
| 30..46 | two trivial `memcpy` helpers |
| **62..433** | **seven vendored Intel tables** |
| **434..1353** | **`realm_binary64_to_bid128`, 919 lines of macro-expanded Intel code** |
| 1358..1848 | the actual `Decimal128` methods — ~490 lines |

**The entire difficulty of this unit is one constructor.** 47 of the 48 exported symbols
are thin wrappers over BID functions that are already linkable — `bid128_add`, `_sub`,
`_mul`, `_div`, `_quantize`, `_from_string`, `_to_string`, `_quiet_equal`, `_quiet_less`,
`_quiet_greater`, `_to_int64_int`, `_to_bid32`, `_to_bid64`, `_from_uint64`,
`bid32_to_bid128`, `bid64_to_bid128`, plus the `__bid_IDEC_glbround` rounding-mode global.
Rust binds those directly and the methods become straightforward.

The 48th is `Decimal128(double, RoundTo)`, and it needs `realm_binary64_to_bid128`.

## Why that function cannot be linked

The comment at `decimal128.cpp:1352` explains the vendoring:

> The following function is effectively 'binary64_to_bid128' from the Intel library. All
> macros are expanded, so it is perhaps not that readable. The reason for including it
> here is to avoid including 'bid_binarydecimal.c' in its entirety as this would add
> around 2Mb of data to the binary.

Measured, and it is worse than "it is a private copy":

- `binary64_to_bid128` is **defined nowhere in this build** and is absent from
  `build/oracle/trace_runner`. The linked Intel objects cover add/mul/div/quantize/string
  and the `bid_round*` helpers; no `binary*` conversion entry point exists at all.
- `realm_binary64_to_bid128` does not appear in `decimal128.cpp.o`'s symbol table **at
  all** — it is in an anonymous namespace and was inlined into its only caller at `-O3`.
  Only its tables survive, as `non-external` symbols.
- Making the library version available would mean adding a source file to the build, which
  is a build change, which this repo forbids.

So Rust must **reimplement** it. Per `.claude/rules/unit-screening.md` step 8 that is the
portable case, not a park: what was inlined is bit-unpacking arithmetic Rust can express,
not a lambda handed to a C++ template (the `column_binary` case).

## The one design decision, recorded before it is made

A faithful transcription is ~700 lines of Rust. A *re-derivation* is about 80: a double is
exactly `m × 2^e`, so exact big-integer arithmetic can produce the decimal and round it to
34 significant digits, which is what the function computes.

**Mirror, do not re-derive.** `format-fidelity.md` says so, and it is right here for a
specific reason: the shorter route is tempting exactly because it is shorter, and the
places the two would diverge — the `pfpsf` flag bits, the two `bid128_quantize` retry
paths, the rounding-mode global — are all invisible to a casual test and visible in a
stored `.realm` byte. The re-derivation is recorded here so the next iteration does not
rediscover it and mistake it for a new idea.

## What is committed here

| file | what it does |
|---|---|
| `extract_block.py` | slices the vendored code out of `decimal128.cpp` into `tables_block.inc` (typedefs + tables) and `conv_block.inc` (those plus the function). Anchors on content, asserts brace balance, refuses to write an unbalanced slice |
| `emit_tables.cpp` | prints the seven tables as decimal, **from the compiler** |
| `emit_conv.cpp` | reads doubles as raw u64 from stdin, prints `bits w0 w1 flags` per line — the oracle half of the conversion differential |
| `gen_doubles.py` | the input corpus: specials, every one of the 2048 exponents at nine significands and both signs, the denormal boundary, exact powers of 10 and 2, the `a == 48` cutoff, 15/17-digit cases, then a seeded random tail |
| `rust_conv_driver.rs` | the Rust half — same stdin/stdout contract as `emit_conv.cpp` |
| `run_conv_differential.sh` | builds both sides and compares them at all five rounding modes |

`crates/realm-core-rs/src/decimal128/tables.rs` is generated by `emit_tables.cpp` and is
committed alongside.

### The tables are generated because parsing them is wrong

The first attempt extracted them from the source text with a regex and got **5 of the 7
row counts wrong** — 76/27/505/228/148 where the compiler says 49/20/49/80/128. Two
independent causes: index annotations in comments look like table data, and several
entries are *expressions* rather than literals (`bid_roundbound_128` contains
`(1ull << 63)`). Hence `emit_tables.cpp`: the compiler is the only thing that knows what
these tables contain.

This is the same lesson as `evidence-and-linkage.md`'s "get the ABI off the built object,
not out of your head", in a register that has nothing to do with ABI.

## Running it

```
./migration/checks/decimal128/run_conv_differential.sh          # 3M doubles, ~14s
N_RANDOM=200000 ./migration/checks/decimal128/run_conv_differential.sh   # quick
```

Needs `make oracle` first, for `librealm.a` — which is on the link line for
`__bid_IDEC_glbround` alone; the conversion code itself comes in textually.

Two build details in that script are load-bearing rather than incidental:

- **`--target x86_64-apple-darwin`, not the host default.** The shell here runs under
  Rosetta and the oracle is x86_64; a native arm64 Rust build links happily and compares
  two different ABIs. This is the silent-green failure recorded in
  `evidence-and-linkage.md`, met again in a new place.
- **`-C overflow-checks=on`.** The C wraps freely at `-O3 -DNDEBUG`; the port is written
  with `wrapping_*` throughout. Building the driver with checks *on* means a missed
  `wrapping_*` panics loudly instead of diverging quietly.

## Step 1 is done: the conversion is ported and verified

`crates/realm-core-rs/src/decimal128/conv.rs` mirrors `realm_binary64_to_bid128`.
`run_conv_differential.sh` compares it against the vendored C++ over the whole corpus at
**every rounding mode**:

```
PASS: C++ and Rust agree on 3045519 doubles x 5 rounding modes = 15227595 comparisons.
```

Value *and* flags, byte for byte. Runtime is about 14 seconds.

### What the controls established

Ten deliberate bugs, each reverted after measuring. Seven bite:

| control | result |
|---|---|
| ×10 correction disabled | 387,871 lines |
| inexact flag never set | 2,924,263 |
| underflow flag never set | 233 |
| invalid flag never set | 79 |
| both exact fast paths disabled | 18,623 |
| `e_out` constant `19728 -> 19727` | 164,918 |
| `e_out` bias `6512 -> 6511` | 44,740 |
| round-bound index drops `glbround` | R1–R4 |
| round compare `<` becomes `<=` | R0/R1/R2 |
| **round-bound index drops the sign term** | **R1/R2 only** |

That last row is why the mode sweep exists. Rows 0,1 of `bid_roundbound_128` are *identical*
to rows 2,3, so at the default rounding mode the sign term cannot matter and the control is
invisible. Rows 4–7 differ, and under directed rounding the same control diverges on
1,455,362 lines. Tested at one rounding mode this port would have shipped with an untested
branch that looked tested.

Note also that disabling the fast paths changes output at all: decimal128 has **redundant
representations** (the same number at different coefficient/exponent pairings), so a
shortcut that returns a different-but-equal encoding is byte-visible. That is precisely the
class of bug this project exists to catch, and it means the fast paths are not optional.

### Three controls that cannot bite, each diagnosed rather than assumed

| control | why |
|---|---|
| `a <= 48` boundary, `<=` becomes `<` | **the behaviours coincide.** A corpus was *constructed* to hit `cint[0] == pow5[0]` exactly — instrumentation confirms 10 hits — and both branches produce identical output there. Not a coverage gap |
| reciprocal reload guard, `+1` becomes `+0`, `+2` or `+256` | **absorbed.** The branch runs 2,405,377 times, but the guard sits in the lowest word of a 256-bit reciprocal and its contribution falls below the precision retained after the 384-bit multiply and shift. This is the one constant the differential cannot pin; it is mirrored faithfully anyway |
| `e_out` correction term `19779 -> 19778` | **self-correcting by design.** The comment at `decimal128.cpp:1245` says the provisional exponent is "either e_out or e_out-1 depending on later significand check", and the ×10 step is that check. Perturbing the *fine* term is absorbed; perturbing the coarse term (`19728`) or the bias (`6512`) bites, so the estimate itself is covered |

### Branch coverage, measured not inferred

Over 345,519 doubles: `a <= 0` fast path 15,042; `a <= 48` fast path 4,345; slow path the
rest; `e_hi != 39` (the 256×256→512 multiply) 2,405,377 of 2,908,465 on the 3M corpus, with
`e_hi` spanning 37..42.

## Step 2: the methods

Ported over the bound BID entry points; see `migration/checks/decimal128_differential.cpp`.
Exclusion is confirmed by fingerprint rather than inference: `decimal128.cpp`'s anonymous-
namespace tables (`bid_power_five`, `bid_coefflimits_bid128`) are present in the pure-C++
driver and the oracle binary, and **absent** from the Rust driver and the hybrid. If the
C++ TU had won the link they would be there.

The methods differential caught a real bug on its first run, on 17 lines whose characters
were identical and whose `capacity()` was not:

> `to_string()`'s `bid128_to_string` path ends in `return std::string(buffer)` — the
> `const char*` **constructor**, whose capacity rule is not the append/growth rule. For a
> 24-character result the constructor gives 31 and growing an empty string by appending
> gives 47.

Fixed by binding `std::string::basic_string(const char*)` rather than reproducing a second
capacity rule, per the guidance the `unicode` port established. The measured constructor
rule is recorded in the source comment and deliberately unused: `n <= 22 -> 22`,
`n == 23 -> 25`, `n >= 24 -> round_up(n + 1, 8) - 1`. Note this is a *third* libc++
capacity rule in this repo, distinct from the `resize` rule measured for `unicode` and the
single fixed length hand-rolled in `object_id`.

### Controls on the methods

Six injected bugs. Four bite: the `to_string` capacity above; `compare()` ordering NaN
last instead of first; `to_bid32` ignoring the `INEXACT` mask; and the `Bid32` equality
exponent cutoff `6 -> 5`.

That last one **only bites after the driver was extended.** The original vectors never
contained two `Bid32` values denoting the same number at exponents differing by exactly
six, so the cutoff was never exercised and lowering it passed. Pairs that straddle 5, 6 and
7, plus significands that trip the `9999999` overflow guard mid-loop, were constructed and
added; the control then bites.

Two do not bite, and both are provably inert rather than uncovered:

| control | why |
|---|---|
| `operator==` drops the `null == null` shortcut | `null` is `{0xaa, 0x7c00…}`, which *is* a NaN. `bid128_quiet_equal` returns 0 for it, and the code then falls into the NaN branch, which compares raw words and returns true. The shortcut is redundant with the branch below it |
| the `int64` constructor uses `wrapping_neg` instead of the C's `val == lowest() ? val : ~val + 1` | `(!x) + 1 == -x` in two's complement for every `x`, `INT64_MIN` included. The C's ternary is a no-op. Mirrored anyway, since that equivalence is the sort of thing that stays true until someone edits it |

Note for step 4: this unit is byte-visible (`Decimal128` is a stored column type) but
**untraced** — the trace schema is int/string/double/bool, so no trace stores a decimal.
`make verify` will prove the link is intact and nothing else regressed; the differential
is the evidence. Same situation as `unicode` and `array_timestamp`.
