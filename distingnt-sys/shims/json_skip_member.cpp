#include <cstddef>
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
extern "C" bool rust_nt_json_skip_member(_NT_jsonParse* self) { return self->skipMember(); }

