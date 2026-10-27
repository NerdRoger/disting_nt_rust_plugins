#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_number_f32(_NT_jsonParse* self, float* value) { return self->number(*value); }

