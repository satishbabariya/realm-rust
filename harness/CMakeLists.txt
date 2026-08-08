cmake_minimum_required(VERSION 3.20)
project(realm_migration_harness CXX C)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Which stack are we building? Set by the Makefile, never guessed here.
#   oracle -> pure C++ realm-core
#   hybrid -> realm-core with ported units served by crates/realm-core-rs
set(HARNESS_STACK "oracle" CACHE STRING "oracle | hybrid")

# ---------------------------------------------------------------------------
# Flag parity.
#
# The oracle records the flag set it was configured with; the hybrid must match
# it exactly. Byte divergence caused by flag drift is indistinguishable from
# byte divergence caused by a bad port, and debugging the former while believing
# the latter is the worst afternoon this project can produce.
# ---------------------------------------------------------------------------
set(REALM_ENABLE_SYNC OFF CACHE BOOL "" FORCE)
set(REALM_BUILD_TEST_CLIENT OFF CACHE BOOL "" FORCE)
set(REALM_ENABLE_ENCRYPTION ON CACHE BOOL "" FORCE)
set(REALM_ENABLE_GEOSPATIAL ON CACHE BOOL "" FORCE)
set(REALM_APP_SERVICES OFF CACHE BOOL "" FORCE)
set(REALM_NO_TESTS ON CACHE BOOL "" FORCE)

# Encryption on Linux/Android needs OpenSSL (upstream/CMakeLists.txt:297) and will
# otherwise try to download a prebuilt tarball from static.realm.io. macOS uses
# SecureTransport and needs nothing.
if(NOT APPLE)
    set(REALM_USE_SYSTEM_OPENSSL ON CACHE BOOL "" FORCE)
endif()

# realm-core's vendored s2 geospatial library specializes std::is_pod
# (upstream/src/external/s2/s1interval.h:189), which clang >= 21 rejects outright:
#   error: 'is_pod' cannot be specialized [-Winvalid-specialization]
# Suppressing the diagnostic is the right fix here rather than patching upstream/,
# which must stay byte-identical to the pinned realm-core. Applied globally so both
# stacks compile the same sources the same way.
include(CheckCXXCompilerFlag)
check_cxx_compiler_flag("-Wno-invalid-specialization" HAVE_WNO_INVALID_SPECIALIZATION)
if(HAVE_WNO_INVALID_SPECIALIZATION)
    add_compile_options(-Wno-invalid-specialization)
endif()

set(HARNESS_FLAG_SIGNATURE
    "sync=${REALM_ENABLE_SYNC};enc=${REALM_ENABLE_ENCRYPTION};geo=${REALM_ENABLE_GEOSPATIAL};app=${REALM_APP_SERVICES};build=${CMAKE_BUILD_TYPE};cxx=${CMAKE_CXX_STANDARD};sys=${CMAKE_SYSTEM_NAME};arch=${CMAKE_SYSTEM_PROCESSOR}"
)
file(WRITE "${CMAKE_BINARY_DIR}/flags.sig" "${HARNESS_FLAG_SIGNATURE}\n")

add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/../upstream" realm-core EXCLUDE_FROM_ALL)

add_executable(trace_runner trace_runner.cpp)
target_include_directories(trace_runner PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/../upstream/src")
target_link_libraries(trace_runner PRIVATE RealmFFIStatic)

# Fault injection, used only by `make fault-check` to prove the gate can still fail.
option(HARNESS_FAULT "Perturb one written integer so diff-test MUST diverge" OFF)
if(HARNESS_FAULT)
    target_compile_definitions(trace_runner PRIVATE HARNESS_FAULT=1)
    message(STATUS "harness: FAULT INJECTION ON — this stack is deliberately wrong")
endif()

if(HARNESS_STACK STREQUAL "hybrid")
    # crates/realm-core-rs builds a staticlib exporting the same C++ symbols for
    # every unit that has been ported. Linked *before* the C++ archives so its
    # definitions win; the C++ TUs it replaces are excluded in that crate's build.rs.
    # RUST_STATICLIB is passed in by the Makefile, which owns the arch/triple
    # decision. Deriving it here as well would create two places that can disagree,
    # and the failure mode of disagreement is silent: ld drops an arch-mismatched
    # archive with only a warning, the hybrid quietly becomes pure C++, and
    # diff-test goes green while proving nothing.
    if(NOT DEFINED RUST_STATICLIB)
        message(FATAL_ERROR
            "harness: hybrid requires -DRUST_STATICLIB=<path>. Build via 'make hybrid'.")
    endif()
    if(EXISTS "${RUST_STATICLIB}")
        target_link_libraries(trace_runner PRIVATE "${RUST_STATICLIB}")
        # ld only pulls an archive member that resolves an undefined symbol. Without
        # a reference, the probe object is never extracted and the "is Rust linked?"
        # check can never pass — so the runner references it under this define.
        target_compile_definitions(trace_runner PRIVATE HARNESS_HYBRID=1)
        message(STATUS "harness: hybrid linking ${RUST_STATICLIB}")
    else()
        message(WARNING
            "harness: ${RUST_STATICLIB} does not exist. The hybrid is therefore "
            "byte-identical to the oracle by construction. Correct before the first "
            "unit is ported; a bug at any point after.")
    endif()
endif()
