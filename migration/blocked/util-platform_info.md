# Parked: `upstream/src/realm/util/platform_info.cpp`

145 lines, two exported symbols, **neither linked**. Parked 2026-08-09 at step 1.

```
nm -g util/platform_info.cpp.o | grep -v ' U '   ->  realm::util::get_platform_info(PlatformInfo&)
                                                     realm::util::PlatformInfo::~PlatformInfo()
# intersect with nm build/oracle/trace_runner    ->  0
```

Category 1 of `evidence-and-linkage.md`: unreachable. It wraps `uname(2)` to fill a
struct of `std::string`s for diagnostics and log banners; nothing in the linked
`Storage` → `ObjectStore` → `RealmFFIStatic` chain calls it.

Also throws (`get_platform_info` owns an EH site, per the owning-function test) and
returns `std::string`s by value, so it would be a step-5 park even if it were reachable.
But step 1 settles it first and at lower cost — this is the sixth unit in the queue whose
position came from line count rather than from whether the gate can see it.

## What would unblock it

A caller. Not a shim.
