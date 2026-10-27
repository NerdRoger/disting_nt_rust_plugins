#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" void rust_nt_json_add_four_cc(_NT_jsonStream* self, uint32_t value) { self->addFourCC(value); }

