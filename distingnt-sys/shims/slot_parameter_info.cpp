#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/slot.h>
extern "C" bool rust_nt_slot_parameter_info(const _NT_slot* self, _NT_parameter* info, uint32_t index) { return self->parameterInfo(*info, index); }

