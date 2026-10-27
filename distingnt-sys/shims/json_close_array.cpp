#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" void rust_nt_json_close_array(_NT_jsonStream* self) { self->closeArray(); }

