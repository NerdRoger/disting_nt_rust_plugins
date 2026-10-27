#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/slot.h>
extern "C" uint32_t rust_nt_slot_parameter_string(const _NT_slot* self, uint32_t index, char* buffer) { return self->parameterString(index, buffer); }

