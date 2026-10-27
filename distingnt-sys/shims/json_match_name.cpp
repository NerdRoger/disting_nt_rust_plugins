#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_match_name(_NT_jsonParse* self, const char* name) { return self->matchName(name); }

