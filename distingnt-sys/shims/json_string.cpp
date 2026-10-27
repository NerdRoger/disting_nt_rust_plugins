#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_string(_NT_jsonParse* self, const char** value) { return self->string(*value); }

