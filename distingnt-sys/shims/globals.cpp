#include <distingnt/api.h>

// Accessing external Disting data directly from LLVM Rust emits a relocation
// the plugin loader cannot handle. GCC emits the supported GOT_BREL form.
extern "C" const _NT_globals* rust_nt_globals()
{
	return &NT_globals;
}
