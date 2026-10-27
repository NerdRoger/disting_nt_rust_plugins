#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" void rust_nt_json_close_object(_NT_jsonStream* self) { self->closeObject(); }

