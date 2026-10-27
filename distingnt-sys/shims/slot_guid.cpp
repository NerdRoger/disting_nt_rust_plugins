#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/slot.h>
extern "C" uint32_t rust_nt_slot_guid(const _NT_slot* self) { return self->guid(); }

