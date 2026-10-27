#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/slot.h>
extern "C" _NT_algorithm* rust_nt_slot_plugin(const _NT_slot* self) { return self->plugin(); }

