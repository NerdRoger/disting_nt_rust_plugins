#include <cstddef>

// RTABI32 memory helpers may clobber integer scratch registers, but must
// preserve VFP registers. Firmware memset has the ordinary C calling convention,
// so save its caller-saved VFP registers before forwarding. Its C callee must
// preserve d8-d15. Keep the stack eight-byte aligned at the call.
// https://github.com/ARM-software/abi-aa/blob/main/rtabi32/rtabi32.rst#memory-clearing-and-setting
extern "C" __attribute__((naked)) void __wrap___aeabi_memclr(void*, std::size_t)
{
    __asm__ volatile(
        "push {r4, lr}\n"
        "vpush {d0-d7}\n"
        "vmrs r4, fpscr\n"
        "mov r2, r1\n"
        "movs r1, #0\n"
        "bl memset\n"
        "vmsr fpscr, r4\n"
        "vpop {d0-d7}\n"
        "pop {r4, pc}\n"
    );
}

extern "C" void __wrap___aeabi_memclr4(void*, std::size_t)
    __attribute__((alias("__wrap___aeabi_memclr")));
extern "C" void __wrap___aeabi_memclr8(void*, std::size_t)
    __attribute__((alias("__wrap___aeabi_memclr")));
