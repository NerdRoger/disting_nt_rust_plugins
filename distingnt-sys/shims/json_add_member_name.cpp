#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" void rust_nt_json_add_member_name(_NT_jsonStream* self, const char* value) { self->addMemberName(value); }

