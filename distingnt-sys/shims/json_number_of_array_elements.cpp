#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_number_of_array_elements(_NT_jsonParse* self, int* value) { return self->numberOfArrayElements(*value); }

