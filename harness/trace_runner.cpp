// trace_runner — drives realm-core through a deterministic operation sequence
// and writes a .realm file. Compiled once per stack (oracle / hybrid); the two
// binaries must produce byte-identical output for the same trace.
//
// Every symbol used here is from upstream/src/realm.h (the stable C API), so the
// runner does not need to change as internals are ported.
//
// Usage: trace_runner <trace-file> <output.realm>

#include <realm.h>

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#ifdef HARNESS_HYBRID
// Presence probe for the hybrid stack. Referencing it forces ld to extract the
// object from librealm_core_rs.a; without a reference the archive member is never
// pulled in and "the Rust is linked" becomes unobservable. `make hybrid` greps the
// linked binary for this symbol and fails the build if it is missing, because a
// silently-unlinked hybrid produces a green diff-test over pure C++.
extern "C" std::uint32_t realm_rs_units_ported();
#endif

namespace {

[[noreturn]] void fail(const std::string& what)
{
    realm_error_t err{};
    if (realm_get_last_error(&err)) {
        std::fprintf(stderr, "trace_runner: %s: realm error %d: %s\n", what.c_str(), int(err.error),
                     err.message ? err.message : "(no message)");
        realm_clear_last_error();
    }
    else {
        std::fprintf(stderr, "trace_runner: %s\n", what.c_str());
    }
    std::exit(1);
}

void check(bool ok, const char* what)
{
    if (!ok)
        fail(what);
}

realm_value_t rv_int(int64_t v)
{
    realm_value_t val{};
    val.type = RLM_TYPE_INT;
#ifdef HARNESS_FAULT
    // Deliberate fault injection, enabled only by `make fault-check`.
    //
    // A gate that has never failed on this machine tells you nothing when it passes.
    // This perturbs one integer by one, which is the smallest realistic porting bug:
    // functionally almost invisible, and byte-visible exactly where element width is
    // decided. `make fault-check` builds a third stack with this on and REQUIRES
    // diff-test to report divergence. If it does not, the harness is not measuring
    // anything and every green result so far is meaningless.
    val.integer = v + 1;
#else
    val.integer = v;
#endif
    return val;
}

realm_value_t rv_bool(bool v)
{
    realm_value_t val{};
    val.type = RLM_TYPE_BOOL;
    val.boolean = v;
    return val;
}

realm_value_t rv_double(double v)
{
    realm_value_t val{};
    val.type = RLM_TYPE_DOUBLE;
    val.dnum = v;
    return val;
}

// The string must outlive the call; callers keep the backing std::string alive.
realm_value_t rv_string(const std::string& s)
{
    realm_value_t val{};
    val.type = RLM_TYPE_STRING;
    val.string.data = s.data();
    val.string.size = s.size();
    return val;
}

// ---------------------------------------------------------------------------
// Fixed schema. Deliberately minimal and deliberately covers the four scalar
// widths that realm packs differently on disk.
//
//   class T { _id: int (pk), i: int, s: string, d: double, b: bool }
// ---------------------------------------------------------------------------

struct Handles {
    realm_t* realm = nullptr;
    realm_class_key_t class_key{};
    realm_property_key_t pk_key{};
    realm_property_key_t i_key{};
    realm_property_key_t s_key{};
    realm_property_key_t d_key{};
    realm_property_key_t b_key{};
};

realm_schema_t* make_schema()
{
    static realm_property_info_t props[5];
    std::memset(props, 0, sizeof(props));

    props[0].name = "_id";
    props[0].public_name = "";
    props[0].type = RLM_PROPERTY_TYPE_INT;
    props[0].collection_type = RLM_COLLECTION_TYPE_NONE;
    props[0].link_target = "";
    props[0].link_origin_property_name = "";
    props[0].flags = RLM_PROPERTY_PRIMARY_KEY;

    auto scalar = [](realm_property_info_t& p, const char* name, realm_property_type_e type) {
        p.name = name;
        p.public_name = "";
        p.type = type;
        p.collection_type = RLM_COLLECTION_TYPE_NONE;
        p.link_target = "";
        p.link_origin_property_name = "";
        p.flags = RLM_PROPERTY_NORMAL;
    };
    scalar(props[1], "i", RLM_PROPERTY_TYPE_INT);
    scalar(props[2], "s", RLM_PROPERTY_TYPE_STRING);
    scalar(props[3], "d", RLM_PROPERTY_TYPE_DOUBLE);
    scalar(props[4], "b", RLM_PROPERTY_TYPE_BOOL);

    static realm_class_info_t cls;
    std::memset(&cls, 0, sizeof(cls));
    cls.name = "T";
    cls.primary_key = "_id";
    cls.num_properties = 5;
    cls.num_computed_properties = 0;
    cls.flags = RLM_CLASS_NORMAL;

    const realm_property_info_t* class_props[1] = {props};
    realm_schema_t* schema = realm_schema_new(&cls, 1, class_props);
    if (!schema)
        fail("realm_schema_new");
    return schema;
}

Handles open_realm(const std::string& path)
{
    realm_schema_t* schema = make_schema();

    realm_config_t* config = realm_config_new();
    if (!config)
        fail("realm_config_new");
    realm_config_set_path(config, path.c_str());
    realm_config_set_schema(config, schema);
    realm_config_set_schema_version(config, 1);
    realm_config_set_schema_mode(config, RLM_SCHEMA_MODE_AUTOMATIC);
    // Notifications and the shared-realm cache introduce thread/scheduler state that
    // has no business influencing the bytes on disk. Off, in both stacks.
    realm_config_set_automatic_change_notifications(config, false);
    realm_config_set_cached(config, false);

    Handles h;
    h.realm = realm_open(config);
    realm_release(config);
    realm_release(schema);
    if (!h.realm)
        fail("realm_open");

    bool found = false;
    realm_class_info_t cls_info{};
    check(realm_find_class(h.realm, "T", &found, &cls_info), "realm_find_class");
    check(found, "class T not found after open");
    h.class_key = cls_info.key;

    auto prop = [&](const char* name) {
        bool ok = false;
        realm_property_info_t info{};
        check(realm_find_property(h.realm, h.class_key, name, &ok, &info), "realm_find_property");
        check(ok, "property not found");
        return info.key;
    };
    h.pk_key = prop("_id");
    h.i_key = prop("i");
    h.s_key = prop("s");
    h.d_key = prop("d");
    h.b_key = prop("b");
    return h;
}

// ---------------------------------------------------------------------------
// Trace language. Line-oriented; '#' and blank lines ignored.
//
//   begin
//   insert <pk:int> <i:int> <s:string-no-spaces> <d:double> <b:0|1>
//   erase  <pk:int>
//   commit
//
// Any parse error or unknown verb is fatal — a trace that silently does nothing
// would pass diff-test while proving nothing.
// ---------------------------------------------------------------------------

void run_trace(Handles& h, std::istream& in)
{
    std::string line;
    int lineno = 0;
    bool in_write = false;

    while (std::getline(in, line)) {
        ++lineno;
        if (auto hash = line.find('#'); hash != std::string::npos)
            line.erase(hash);
        std::istringstream ls(line);
        std::string verb;
        if (!(ls >> verb))
            continue;

        if (verb == "begin") {
            check(!in_write, "nested begin");
            check(realm_begin_write(h.realm), "realm_begin_write");
            in_write = true;
        }
        else if (verb == "commit") {
            check(in_write, "commit without begin");
            check(realm_commit(h.realm), "realm_commit");
            in_write = false;
        }
        else if (verb == "insert") {
            check(in_write, "insert outside write transaction");
            int64_t pk = 0, i = 0;
            std::string s;
            double d = 0;
            int b = 0;
            if (!(ls >> pk >> i >> s >> d >> b))
                fail("malformed insert at line " + std::to_string(lineno));
            // Fields are whitespace-delimited, so the empty string needs a spelling.
            // `""` is it; a zero-length column is not the same thing as a narrow one
            // and both need to be reachable from a trace.
            if (s == "\"\"")
                s.clear();

            realm_object_t* obj = realm_object_create_with_primary_key(h.realm, h.class_key, rv_int(pk));
            if (!obj)
                fail("realm_object_create_with_primary_key at line " + std::to_string(lineno));
            check(realm_set_value(obj, h.i_key, rv_int(i), false), "set i");
            check(realm_set_value(obj, h.s_key, rv_string(s), false), "set s");
            check(realm_set_value(obj, h.d_key, rv_double(d), false), "set d");
            check(realm_set_value(obj, h.b_key, rv_bool(b != 0), false), "set b");
            realm_release(obj);
        }
        else if (verb == "erase") {
            check(in_write, "erase outside write transaction");
            int64_t pk = 0;
            if (!(ls >> pk))
                fail("malformed erase at line " + std::to_string(lineno));
            bool found = false;
            realm_object_t* obj = realm_object_find_with_primary_key(h.realm, h.class_key, rv_int(pk), &found);
            if (!found || !obj)
                fail("erase: object not found at line " + std::to_string(lineno));
            check(realm_object_delete(obj), "realm_object_delete");
            realm_release(obj);
        }
        else {
            fail("unknown verb '" + verb + "' at line " + std::to_string(lineno));
        }
    }

    check(!in_write, "trace ended inside a write transaction");
}

// ---------------------------------------------------------------------------
// --open mode: open an existing (usually legacy) realm without a declared schema
// and print a fingerprint of what was found.
//
// Legacy files need a file-format upgrade, which happens in place on open. That
// makes this a stronger check than it first appears: format-compat runs it on two
// copies of the same fixture, one per stack, and byte-compares the *upgraded*
// files. A port that reads old formats but rewrites them differently fails here
// and nowhere else.
// ---------------------------------------------------------------------------
int open_existing(const std::string& path)
{
    realm_config_t* config = realm_config_new();
    if (!config)
        fail("realm_config_new");
    realm_config_set_path(config, path.c_str());
    realm_config_set_schema_mode(config, RLM_SCHEMA_MODE_ADDITIVE_DISCOVERED);
    realm_config_set_automatic_change_notifications(config, false);
    realm_config_set_cached(config, false);

    realm_t* realm = realm_open(config);
    realm_release(config);
    if (!realm)
        fail("realm_open (existing " + path + ")");

    std::printf("schema_version=%llu\n", (unsigned long long)realm_get_schema_version(realm));

    const size_t n = realm_get_num_classes(realm);
    std::printf("num_classes=%zu\n", n);

    std::vector<realm_class_key_t> keys(n);
    size_t got = 0;
    check(realm_get_class_keys(realm, keys.data(), n, &got), "realm_get_class_keys");

    // Sorted by name so the fingerprint does not depend on class-key assignment
    // order, which is an implementation detail rather than a format guarantee.
    std::vector<std::string> lines;
    for (size_t i = 0; i < got; ++i) {
        realm_class_info_t info{};
        check(realm_get_class(realm, keys[i], &info), "realm_get_class");
        size_t count = 0;
        check(realm_get_num_objects(realm, keys[i], &count), "realm_get_num_objects");
        lines.push_back(std::string("class ") + info.name + " properties=" + std::to_string(info.num_properties) +
                        " objects=" + std::to_string(count));
    }
    std::sort(lines.begin(), lines.end());
    for (const auto& l : lines)
        std::printf("%s\n", l.c_str());

    check(realm_close(realm), "realm_close");
    realm_release(realm);
    return 0;
}

} // namespace

int main(int argc, char** argv)
{
    if (argc == 3 && std::strcmp(argv[1], "--open") == 0)
        return open_existing(argv[2]);

    if (argc != 3) {
        std::fprintf(stderr, "usage: trace_runner <trace-file> <output.realm>\n"
                             "       trace_runner --open <existing.realm>\n");
        return 2;
    }
    const std::string trace_path = argv[1];
    const std::string out_path = argv[2];

    // realm's default logger writes compaction timings to stderr. Timings vary per
    // run, so leaving it on makes every diff of captured output look like a failure.
    realm_set_log_level(RLM_LOG_LEVEL_OFF);

#ifdef HARNESS_HYBRID
    // stderr, never stdout: format-compat diffs stdout between the two stacks, and
    // this line exists only in the hybrid.
    std::fprintf(stderr, "harness: rust units ported = %u\n", realm_rs_units_ported());
#endif

    // Start from a clean file every run; a stale file would make byte-comparison
    // depend on run order.
    realm_delete_files(out_path.c_str(), nullptr);

    std::ifstream trace(trace_path);
    if (!trace)
        fail("cannot open trace " + trace_path);

    Handles h = open_realm(out_path);
    run_trace(h, trace);

    // Compact before closing so the file has no allocator slack whose size depends
    // on transient allocation order rather than on the committed data.
    check(realm_compact(h.realm, nullptr), "realm_compact");
    check(realm_close(h.realm), "realm_close");
    realm_release(h.realm);
    return 0;
}
