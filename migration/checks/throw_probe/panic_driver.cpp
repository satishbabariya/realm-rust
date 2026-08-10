#include <cstdio>
extern "C" int rust_panics(int);
int main(){
  try {
    int r = rust_panics(99);
    printf("FAIL: rust_panics returned %d -- the panic was not fatal\n", r);
    return 1;
  } catch (...) {
    printf("FAIL: C++ swallowed a Rust panic via catch (...)\n");
    return 1;
  }
}
