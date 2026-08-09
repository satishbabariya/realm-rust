# Parked: `upstream/src/realm/util/json_parser.cpp`

193 lines, seven realm symbols, **none linked**. Parked 2026-08-09 at step 1.

```
nm -g util/json_parser.cpp.o | grep -v ' U ' | grep '^__ZN.*5realm' | sort -u
# intersect with nm build/oracle/trace_runner  ->  0 of 7
```

Category 1 of `evidence-and-linkage.md`: unreachable. The JSON parser is used by the
sync/app-services code and by BSON, both of which are dead with
`REALM_ENABLE_SYNC=OFF` and `REALM_APP_SERVICES=OFF`. `JSONParser` symbols appear ten
times across `librealm.a`, so it is not dead source — it is dead *in this build
configuration*, which is the same transitive-deadness pattern as
`util/bson/regular_expression`.

A second, independent ground: it is the sole definer of
`realm::util::JSONParser::ErrorCategory`'s vtable, typeinfo and typeinfo-name
(definer count 1), so it would also be a step-2 park. Step 1 settles it first and at
lower cost.

## What would unblock it

A build with sync or app-services enabled — which hard rule 3 (`CLAUDE.md`) puts out of
scope, since flag drift between the stacks produces byte divergence indistinguishable
from a porting bug. Seven other units are parked on exactly this decision.
