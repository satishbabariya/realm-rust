// Emits `realm_binary64_to_bid128(x)` for a stream of doubles read from stdin as raw
// u64 bit patterns. The function lives in an anonymous namespace in decimal128.cpp and
// was inlined away at -O3, so it cannot be linked -- it is textually included instead.
#include <external/IntelRDFPMathLib20U2/LIBRARY/src/bid_conf.h>
#include <external/IntelRDFPMathLib20U2/LIBRARY/src/bid_functions.h>
#include <cstdio>
#include <cstdint>
#include <cstring>
namespace realm { namespace {
#include "conv_block.inc"
} }
int main(){
  uint64_t bits;
  while (fread(&bits, 8, 1, stdin) == 1) {
    double x; std::memcpy(&x, &bits, 8);
    BID_UINT128 out; unsigned flags = 0;
    realm::realm_binary64_to_bid128(&out, &x, &flags);
    printf("%016llx %016llx %016llx %u\n", (unsigned long long)bits,
           (unsigned long long)out.w[0], (unsigned long long)out.w[1], flags);
  }
  return 0;
}
