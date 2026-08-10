#include <realm/exceptions.hpp>
#include <cstdio>
extern "C" void rust_throws_logic_error(int code, const char* msg, size_t len);
int main(){
  const char* m = "Trying to modify database while in read transaction";
  try {
    rust_throws_logic_error(1015, m, __builtin_strlen(m));
    printf("FAIL: no exception propagated\n");
    return 1;
  }
  catch (const realm::LogicError& e) {
    printf("caught LogicError: code=%d what=%s\n", int(e.code()), e.what());
    return 0;
  }
  catch (const std::exception& e) {
    printf("FAIL: caught as std::exception only: %s\n", e.what());
    return 1;
  }
  catch (...) { printf("FAIL: caught unknown\n"); return 1; }
}
