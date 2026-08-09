// Emit realm's vendored BID tables as decimal, straight from the compiler.
// Parsing them out of the C source is unsafe: several entries are expressions
// like (1ull << 63), not literals.
#include <external/IntelRDFPMathLib20U2/LIBRARY/src/bid_conf.h>
#include <external/IntelRDFPMathLib20U2/LIBRARY/src/bid_functions.h>
#include <cstdio>
#include <cstdint>
namespace realm { namespace {
#include "tables_block.inc"
} }
using namespace realm;
#define EMIT128(t) do { printf("@%s %zu 2\n", #t, sizeof(t)/sizeof(t[0])); \
  for (size_t i=0;i<sizeof(t)/sizeof(t[0]);++i) printf("%llu %llu\n",(unsigned long long)t[i].w[0],(unsigned long long)t[i].w[1]); } while(0)
#define EMIT256(t) do { printf("@%s %zu 4\n", #t, sizeof(t)/sizeof(t[0])); \
  for (size_t i=0;i<sizeof(t)/sizeof(t[0]);++i) printf("%llu %llu %llu %llu\n",(unsigned long long)t[i].w[0],(unsigned long long)t[i].w[1],(unsigned long long)t[i].w[2],(unsigned long long)t[i].w[3]); } while(0)
#define EMITINT(t) do { printf("@%s %zu 1\n", #t, sizeof(t)/sizeof(t[0])); \
  for (size_t i=0;i<sizeof(t)/sizeof(t[0]);++i) printf("%d\n", t[i]); } while(0)
int main(){
  EMIT128(bid_coefflimits_bid128);
  EMIT128(bid_roundbound_128);
  EMIT128(bid_power_five);
  EMIT256(bid_outertable_sig);
  EMIT256(bid_innertable_sig);
  EMITINT(bid_outertable_exp);
  EMITINT(bid_innertable_exp);
  return 0;
}
