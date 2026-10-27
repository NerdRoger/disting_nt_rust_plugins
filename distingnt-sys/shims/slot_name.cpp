#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/slot.h>
extern "C" const char* rust_nt_slot_name(const _NT_slot* self) { return self->name(); }

