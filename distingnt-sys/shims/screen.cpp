#include <distingnt/api.h>

// Kept separate so this archive member is linked only when NT_screen is used.
extern "C" uint8_t* rust_nt_screen()
{
	return NT_screen;
}
