#include <cstddef>

// Like memory_clear.cpp, preserve the stronger RTABI32 register contract when
// calling the firmware's ordinary C library. memcpy supports arbitrary alignment;
// the aligned variants may safely share this implementation. No overlap is
// permitted, matching both memcpy and the compiler's __aeabi_memcpy helpers.
extern "C" __attribute__((naked)) void __wrap___aeabi_memcpy(void*, const void*, std::size_t)
{
    __asm__ volatile(
        "push {r4, lr}\n"
        "vpush {d0-d7}\n"
        "vmrs r4, fpscr\n"
        "bl memcpy\n"
        "vmsr fpscr, r4\n"
        "vpop {d0-d7}\n"
        "pop {r4, pc}\n"
    );
}

extern "C" void __wrap___aeabi_memcpy4(void*, const void*, std::size_t)
    __attribute__((alias("__wrap___aeabi_memcpy")));
extern "C" void __wrap___aeabi_memcpy8(void*, const void*, std::size_t)
    __attribute__((alias("__wrap___aeabi_memcpy")));
