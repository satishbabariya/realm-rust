SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

ROOT        := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
UPSTREAM    := $(ROOT)/upstream
HARNESS     := $(ROOT)/harness
BUILD       := $(ROOT)/build
ORACLE_DIR  := $(BUILD)/oracle
HYBRID_DIR  := $(BUILD)/hybrid
FAULT_DIR   := $(BUILD)/fault
OUT         := $(BUILD)/out
MIGRATION   := $(ROOT)/migration
CORPUS      := $(MIGRATION)/corpus
COMPARE     := python3 $(HARNESS)/compare_realm.py
TRACES      := $(sort $(wildcard $(HARNESS)/traces/*.trace))
JOBS        ?= $(shell sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)
BUILD_TYPE  ?= Release

# Architecture parity between the two toolchains.
#
# On this machine the shell runs under Rosetta (uname -m = x86_64) while rustc is a
# native aarch64 toolchain. CMake follows the shell; cargo follows rustc. The result
# is an arch-mismatched staticlib that ld DROPS WITH ONLY A WARNING -- the hybrid
# silently becomes pure C++ and diff-test passes while testing nothing.
#
# So the shell arch is the single source of truth, and cargo is told to match it.
HOST_ARCH   := $(shell uname -m)
ifeq ($(HOST_ARCH),arm64)
RUST_TARGET ?= aarch64-apple-darwin
else ifeq ($(HOST_ARCH),x86_64)
RUST_TARGET ?= x86_64-apple-darwin
else
RUST_TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')
endif
RUST_LIB    := $(ROOT)/target/$(RUST_TARGET)/release/librealm_core_rs.a

# The symbol the probe in crates/realm-core-rs/src/lib.rs exports. Its presence in
# the linked binary is the only proof the Rust actually made it in.
RUST_PROBE  := realm_rs_units_ported

# Colour only when attached to a terminal, so logs stay greppable.
ifneq ($(shell test -t 1 && echo tty),)
  R := \033[31m
  G := \033[32m
  Y := \033[33m
  N := \033[0m
else
  R :=
  G :=
  Y :=
  N :=
endif

.PHONY: help doctor oracle hybrid harness traces determinism-check diff-test \
        format-compat verify corpus shim-report clean clean-out rust fault-check

help:
	@echo "realm-rust migration harness"
	@echo
	@echo "  make doctor             preflight: toolchain, submodules, layout"
	@echo "  make oracle             build the C++ reference stack   (cold: 10-20 min)"
	@echo "  make hybrid             build the Rust-hybrid stack"
	@echo "  make determinism-check  oracle vs oracle — is the format even deterministic?"
	@echo "  make diff-test          oracle vs hybrid, every trace — THE GATE"
	@echo "  make format-compat      both stacks open every legacy realm in migration/corpus/"
	@echo "  make fault-check        prove the gate can FAIL (build a deliberately wrong stack)"
	@echo "  make verify             everything above, in order"
	@echo
	@echo "  make corpus             seed migration/corpus/ from upstream test fixtures"
	@echo "  make shim-report        how much C++ the hybrid still depends on"
	@echo "  make clean              remove build/"

# ---------------------------------------------------------------------------
# doctor — fail loudly and early, with the fix, rather than letting cmake fail
# 400 lines later with something less actionable.
# ---------------------------------------------------------------------------
doctor:
	@ok=0; \
	for t in cmake ninja python3 cargo rustc; do \
	  if command -v $$t >/dev/null 2>&1; then \
	    printf "  $(G)ok$(N)   %-8s %s\n" "$$t" "$$($$t --version 2>&1 | head -1)"; \
	  else \
	    printf "  $(R)MISS$(N) %-8s not on PATH\n" "$$t"; ok=1; \
	  fi; \
	done; \
	if [ ! -f "$(UPSTREAM)/CMakeLists.txt" ]; then \
	  printf "  $(R)MISS$(N) upstream/  -> git submodule update --init --recursive\n"; ok=1; \
	else \
	  printf "  $(G)ok$(N)   upstream %s\n" "$$(git -C $(UPSTREAM) describe --tags 2>/dev/null || echo '(detached)')"; \
	fi; \
	for s in src/external/sha-1 src/external/sha-2; do \
	  if [ -z "$$(ls -A $(UPSTREAM)/$$s 2>/dev/null)" ]; then \
	    printf "  $(R)MISS$(N) upstream/%s is empty -> git submodule update --init --recursive\n" "$$s"; ok=1; \
	  else \
	    printf "  $(G)ok$(N)   upstream/%s\n" "$$s"; \
	  fi; \
	done; \
	if [ "$$(uname -s)" != "Darwin" ] && ! pkg-config --exists openssl 2>/dev/null; then \
	  printf "  $(Y)warn$(N) no system OpenSSL; encryption on Linux needs it (upstream/CMakeLists.txt:297)\n"; \
	fi; \
	if [ $$(ls $(HARNESS)/traces/*.trace 2>/dev/null | wc -l) -eq 0 ]; then \
	  printf "  $(R)MISS$(N) no traces in harness/traces/\n"; ok=1; \
	else \
	  printf "  $(G)ok$(N)   %s traces\n" "$$(ls $(HARNESS)/traces/*.trace | wc -l | tr -d ' ')"; \
	fi; \
	exit $$ok

# ---------------------------------------------------------------------------
# Builds
# ---------------------------------------------------------------------------
$(ORACLE_DIR)/build.ninja:
	@echo "==> configuring oracle"
	@cmake -S $(HARNESS) -B $(ORACLE_DIR) -G Ninja \
	   -DCMAKE_BUILD_TYPE=$(BUILD_TYPE) -DHARNESS_STACK=oracle

oracle: $(ORACLE_DIR)/build.ninja
	@echo "==> building oracle (-j$(JOBS))"
	@cmake --build $(ORACLE_DIR) --target trace_runner -j $(JOBS)
	@cp $(ORACLE_DIR)/flags.sig $(BUILD)/oracle-flags.sig
	@echo "$(G)oracle ready$(N): $(ORACLE_DIR)/trace_runner"

rust:
	@if [ ! -f $(ROOT)/crates/realm-core-rs/Cargo.toml ]; then \
	  echo "$(Y)no Rust crate yet — hybrid will be identical to oracle$(N)"; exit 0; \
	fi; \
	if ! rustup target list --installed 2>/dev/null | grep -qx "$(RUST_TARGET)"; then \
	  echo "==> installing rust target $(RUST_TARGET) (must match the $(HOST_ARCH) shell)"; \
	  rustup target add $(RUST_TARGET); \
	fi; \
	echo "==> building crates/realm-core-rs for $(RUST_TARGET)"; \
	cargo build --release --target $(RUST_TARGET) --manifest-path $(ROOT)/Cargo.toml

$(HYBRID_DIR)/build.ninja:
	@echo "==> configuring hybrid"
	@cmake -S $(HARNESS) -B $(HYBRID_DIR) -G Ninja \
	   -DCMAKE_BUILD_TYPE=$(BUILD_TYPE) -DHARNESS_STACK=hybrid \
	   -DRUST_STATICLIB=$(RUST_LIB)

hybrid: rust $(HYBRID_DIR)/build.ninja
	@# Flag parity guard. Drift between the stacks produces byte divergence that
	@# is indistinguishable from a porting bug, and costs days to attribute.
	@if [ -f $(BUILD)/oracle-flags.sig ] && ! diff -q $(BUILD)/oracle-flags.sig $(HYBRID_DIR)/flags.sig >/dev/null; then \
	  echo "$(R)FLAG DRIFT$(N) — hybrid was configured differently from the oracle:"; \
	  diff $(BUILD)/oracle-flags.sig $(HYBRID_DIR)/flags.sig || true; \
	  echo "Reconfigure both (make clean && make oracle hybrid) before trusting any diff-test."; \
	  exit 1; \
	fi
	@echo "==> building hybrid (-j$(JOBS))"
	@cmake --build $(HYBRID_DIR) --target trace_runner -j $(JOBS)
	@# Link-drop guard. ld ignores an arch-mismatched or malformed archive with a
	@# warning and exits 0, leaving a "hybrid" that is pure C++. That produces a
	@# green diff-test that proves nothing, which is worse than a red one.
	@if [ -f $(RUST_LIB) ] && ! nm -g $(HYBRID_DIR)/trace_runner 2>/dev/null | grep -q "$(RUST_PROBE)"; then \
	  echo "$(R)RUST NOT LINKED$(N) — $(RUST_LIB) exists but $(RUST_PROBE) is absent from the binary."; \
	  echo "  ld almost certainly dropped the archive. Check architecture agreement:"; \
	  echo "    binary:    $$(lipo -info $(HYBRID_DIR)/trace_runner 2>/dev/null | sed 's/.*: //')"; \
	  echo "    staticlib: $$(lipo -info $(RUST_LIB) 2>/dev/null | sed 's/.*: //')"; \
	  echo "  A diff-test run in this state would pass while testing pure C++."; \
	  exit 1; \
	fi
	@echo "$(G)hybrid ready$(N): $(HYBRID_DIR)/trace_runner ($(RUST_TARGET))"

harness: oracle

traces:
	@for t in $(TRACES); do echo "  $$(basename $$t)"; done

# ---------------------------------------------------------------------------
# determinism-check — runs BEFORE any Rust exists, and gates everything.
#
# If realm-core does not produce byte-identical output for the same operation
# sequence run twice, byte-identity is not a usable acceptance criterion and the
# entire strategy needs rethinking. Better to learn that in minute five.
# ---------------------------------------------------------------------------
determinism-check: oracle
	@mkdir -p $(OUT)
	@fail=0; \
	for t in $(TRACES); do \
	  n=$$(basename $$t .trace); \
	  $(ORACLE_DIR)/trace_runner $$t $(OUT)/$$n.a.realm; \
	  $(ORACLE_DIR)/trace_runner $$t $(OUT)/$$n.b.realm; \
	  if $(COMPARE) $(OUT)/$$n.a.realm $(OUT)/$$n.b.realm --label-a run1 --label-b run2 >/dev/null 2>&1; then \
	    printf "  $(G)det$(N)  %s\n" "$$n"; \
	  else \
	    printf "  $(R)NON-DETERMINISTIC$(N) %s\n" "$$n"; \
	    $(COMPARE) $(OUT)/$$n.a.realm $(OUT)/$$n.b.realm --label-a run1 --label-b run2 || true; \
	    fail=1; \
	  fi; \
	done; \
	if [ $$fail -ne 0 ]; then \
	  echo; \
	  echo "$(R)The oracle is not deterministic for these traces.$(N)"; \
	  echo "Byte-identity cannot be the acceptance criterion until this is understood."; \
	  echo "Either the trace admits nondeterminism (fix the trace) or the format embeds"; \
	  echo "run-varying state (narrow the comparison). Do not port anything until then."; \
	fi; \
	exit $$fail

# ---------------------------------------------------------------------------
# diff-test — the real gate.
# ---------------------------------------------------------------------------
diff-test: oracle hybrid
	@mkdir -p $(OUT)
	@fail=0; \
	for t in $(TRACES); do \
	  n=$$(basename $$t .trace); \
	  $(ORACLE_DIR)/trace_runner $$t $(OUT)/$$n.oracle.realm; \
	  $(HYBRID_DIR)/trace_runner $$t $(OUT)/$$n.hybrid.realm; \
	  if $(COMPARE) $(OUT)/$$n.oracle.realm $(OUT)/$$n.hybrid.realm --label-a oracle --label-b hybrid >/dev/null 2>&1; then \
	    printf "  $(G)pass$(N) %s\n" "$$n"; \
	  else \
	    printf "  $(R)FAIL$(N) %s\n" "$$n"; \
	    $(COMPARE) $(OUT)/$$n.oracle.realm $(OUT)/$$n.hybrid.realm --label-a oracle --label-b hybrid || true; \
	    fail=1; \
	  fi; \
	done; \
	exit $$fail

# ---------------------------------------------------------------------------
# format-compat — can both stacks still read files written by older realm-core?
# A port that only round-trips its own output has not preserved the format.
# ---------------------------------------------------------------------------
corpus:
	@mkdir -p $(CORPUS)
	@n=0; \
	while IFS= read -r f; do \
	  cp "$$f" $(CORPUS)/ 2>/dev/null && n=$$((n+1)) || true; \
	done < <(find $(UPSTREAM)/test -name '*.realm' -not -name '*decrypt*' 2>/dev/null); \
	echo "seeded $$n legacy realm files into migration/corpus/"; \
	echo "(files with 'decrypt' in the name are excluded: encrypted realms cannot be byte-compared)"

format-compat: oracle hybrid
	@if [ -z "$$(ls -A $(CORPUS) 2>/dev/null)" ]; then \
	  echo "$(Y)corpus empty — run 'make corpus' first$(N)"; exit 0; \
	fi
	@mkdir -p $(OUT)/compat
	@fail=0; \
	for f in $(CORPUS)/*.realm; do \
	  n=$$(basename $$f .realm); \
	  cp $$f $(OUT)/compat/$$n.oracle.realm; \
	  cp $$f $(OUT)/compat/$$n.hybrid.realm; \
	  o=0; h=0; \
	  $(ORACLE_DIR)/trace_runner --open $(OUT)/compat/$$n.oracle.realm >$(OUT)/compat/$$n.oracle.txt 2>/dev/null || o=$$?; \
	  $(HYBRID_DIR)/trace_runner --open $(OUT)/compat/$$n.hybrid.realm >$(OUT)/compat/$$n.hybrid.txt 2>/dev/null || h=$$?; \
	  if [ "$$o" != "$$h" ]; then \
	    printf "  $(R)FAIL$(N) %-44s oracle exit=%s hybrid exit=%s\n" "$$n" "$$o" "$$h"; fail=1; \
	  elif [ "$$o" != "0" ]; then \
	    printf "  $(Y)skip$(N) %-44s both stacks reject it (exit %s) — agreement is the check\n" "$$n" "$$o"; \
	  elif ! diff -q $(OUT)/compat/$$n.oracle.txt $(OUT)/compat/$$n.hybrid.txt >/dev/null; then \
	    printf "  $(R)FAIL$(N) %-44s schema fingerprints differ\n" "$$n"; \
	    diff $(OUT)/compat/$$n.oracle.txt $(OUT)/compat/$$n.hybrid.txt || true; fail=1; \
	  elif ! $(COMPARE) $(OUT)/compat/$$n.oracle.realm $(OUT)/compat/$$n.hybrid.realm --label-a oracle --label-b hybrid >/dev/null 2>&1; then \
	    printf "  $(R)FAIL$(N) %-44s upgraded files diverge\n" "$$n"; \
	    $(COMPARE) $(OUT)/compat/$$n.oracle.realm $(OUT)/compat/$$n.hybrid.realm --label-a oracle --label-b hybrid || true; fail=1; \
	  else \
	    printf "  $(G)ok$(N)   %s\n" "$$n"; \
	  fi; \
	done; \
	exit $$fail

# ---------------------------------------------------------------------------
# fault-check — the check on the checker.
#
# Builds a third stack with one written integer deliberately off by one, and
# REQUIRES diff-test to catch it. A gate that has never failed on this machine
# proves nothing when it passes; this is what converts "all green" from a hope
# into evidence.
# ---------------------------------------------------------------------------
fault-check: oracle
	@echo "==> building deliberately-wrong stack"
	@cmake -S $(HARNESS) -B $(FAULT_DIR) -G Ninja \
	   -DCMAKE_BUILD_TYPE=$(BUILD_TYPE) -DHARNESS_STACK=oracle -DHARNESS_FAULT=ON >/dev/null
	@cmake --build $(FAULT_DIR) --target trace_runner -j $(JOBS) >/dev/null
	@mkdir -p $(OUT)/fault
	@caught=0; total=0; \
	for t in $(TRACES); do \
	  n=$$(basename $$t .trace); total=$$((total+1)); \
	  $(ORACLE_DIR)/trace_runner $$t $(OUT)/fault/$$n.good.realm 2>/dev/null; \
	  $(FAULT_DIR)/trace_runner $$t $(OUT)/fault/$$n.bad.realm 2>/dev/null; \
	  if $(COMPARE) $(OUT)/fault/$$n.good.realm $(OUT)/fault/$$n.bad.realm >/dev/null 2>&1; then \
	    printf "  $(R)NOT CAUGHT$(N) %s — injected fault produced an identical file\n" "$$n"; \
	  else \
	    printf "  $(G)caught$(N) %s\n" "$$n"; caught=$$((caught+1)); \
	  fi; \
	done; \
	echo; \
	if [ $$caught -eq 0 ]; then \
	  echo "$(R)THE GATE IS BLIND$(N) — a deliberately wrong stack passed every trace."; \
	  echo "Every green diff-test so far is meaningless. Fix the harness before porting."; \
	  exit 1; \
	fi; \
	echo "$(G)gate is live$(N): $$caught/$$total traces detected the injected fault."; \
	if [ $$caught -lt $$total ]; then \
	  echo "$(Y)Note:$(N) $$((total-caught)) trace(s) did not detect it. That is expected for"; \
	  echo "traces that write no integers through the faulted path, but if a trace you"; \
	  echo "expected to be sensitive is in that set, it is weaker than you think."; \
	fi

verify: doctor oracle hybrid determinism-check diff-test format-compat fault-check
	@echo
	@echo "$(G)VERIFY PASSED$(N) — oracle deterministic, hybrid byte-identical, corpus agrees."

# ---------------------------------------------------------------------------
# shim-report — the health metric. Should rise early, then fall.
# Rising across two consecutive reflection passes means the port is spreading
# rather than converging; stop adding units and consolidate.
# ---------------------------------------------------------------------------
shim-report:
	@total=$$(find $(UPSTREAM)/src/realm -name '*.cpp' | wc -l | tr -d ' '); \
	ported=$$(find $(ROOT)/crates -name '*.rs' 2>/dev/null | wc -l | tr -d ' '); \
	shims=$$(grep -rn "TODO(shim)\|unimplemented!\|extern \"C\"" $(ROOT)/crates 2>/dev/null | wc -l | tr -d ' '); \
	echo "  C++ translation units in upstream/src/realm : $$total"; \
	echo "  Rust source files in crates/                : $$ported"; \
	echo "  shim / extern-C boundary points             : $$shims"; \
	echo; \
	echo "  Watch the third number, not the second. A boundary count that grows for"; \
	echo "  two reflection passes running means the port is spreading, not converging."

clean-out:
	@rm -rf $(OUT)

clean:
	@rm -rf $(BUILD)
	@echo "removed build/"
