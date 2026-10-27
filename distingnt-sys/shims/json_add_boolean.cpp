#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" void rust_nt_json_add_boolean(_NT_jsonStream* self, bool value) { self->addBoolean(value); }

