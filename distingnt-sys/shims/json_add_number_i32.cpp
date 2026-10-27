#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" void rust_nt_json_add_number_i32(_NT_jsonStream* self, int value) { self->addNumber(value); }

