#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_parse_null(_NT_jsonParse* self) { return self->null(); }

