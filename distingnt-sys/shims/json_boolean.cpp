#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_boolean(_NT_jsonParse* self, bool* value) { return self->boolean(*value); }

