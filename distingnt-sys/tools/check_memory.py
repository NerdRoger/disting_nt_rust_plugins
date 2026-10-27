"""Execute ARM runtime bridges with Unicorn (optional: pip install unicorn).

The simulated C library deliberately clobbers VFP scratch registers/FPSCR.
This tests the real compiled ARM instructions, independently of Rust host tests.
"""
from pathlib import Path
import subprocess
import tempfile

from unicorn import Uc, UC_ARCH_ARM, UC_MODE_THUMB, UC_HOOK_CODE
from unicorn import arm_const as reg


def run(*args):
    return subprocess.check_output(args, text=True)


def main():
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="nt-memory-") as directory:
        work = Path(directory)
        flags = ["-mcpu=cortex-m7", "-mthumb", "-mfpu=fpv5-d16", "-mfloat-abi=hard", "-Os", "-fPIC"]
        objects = []
        for name in ["memory_clear", "memory_copy"]:
            obj = work / (name + ".o")
            run("arm-none-eabi-c++", *flags, "-c", str(root / "shims" / (name + ".cpp")), "-o", str(obj))
            objects.append(str(obj))
        stub = work / "firmware.S"
        stub.write_text(""".syntax unified
.thumb
.text
.global memset
.type memset, %function
.thumb_func
memset: bx lr
.global memcpy
.type memcpy, %function
.thumb_func
memcpy: bx lr
.section .note.GNU-stack,"",%progbits
""")
        stub_obj = work / "firmware.o"
        run("arm-none-eabi-c++", *flags, "-c", str(stub), "-o", str(stub_obj))
        elf, binary = work / "memory.elf", work / "memory.bin"
        run("arm-none-eabi-ld", "-Ttext=0x10000", "-e", "__wrap___aeabi_memclr", "-o", str(elf), *objects, str(stub_obj))
        run("arm-none-eabi-objcopy", "-O", "binary", "--only-section=.text", str(elf), str(binary))
        symbols = {}
        for line in run("arm-none-eabi-nm", str(elf)).splitlines():
            parts = line.split()
            if len(parts) == 3:
                symbols[parts[2]] = int(parts[0], 16)

        # Execute the shared Thumb-2/VFP instructions on an ARMv7 VFP model.
        # This verifies the ABI bridge, not Cortex-M interrupt/lazy-FP handling.
        cpu = Uc(UC_ARCH_ARM, UC_MODE_THUMB)
        cpu.ctl_set_cpu_model(reg.UC_CPU_ARM_CORTEX_A9)
        cpu.reg_write(reg.UC_ARM_REG_C1_C0_2, 0xF << 20)
        cpu.reg_write(reg.UC_ARM_REG_FPEXC, 1 << 30)
        for address in [0x10000, 0x30000, 0x40000]:
            cpu.mem_map(address, 0x10000)
        cpu.mem_write(0x10000, binary.read_bytes())
        fp = [getattr(reg, "UC_ARM_REG_S" + str(i)) for i in range(32)]
        saved = [getattr(reg, "UC_ARM_REG_R" + str(i)) for i in range(4, 12)]
        called = []

        def firmware(machine, address, size, _):
            if address not in [symbols["memset"], symbols["memcpy"]]:
                return
            assert machine.reg_read(reg.UC_ARM_REG_SP) % 8 == 0
            dest = machine.reg_read(reg.UC_ARM_REG_R0)
            arg = machine.reg_read(reg.UC_ARM_REG_R1)
            length = machine.reg_read(reg.UC_ARM_REG_R2)
            if address == symbols["memset"]:
                assert arg == 0
                data = bytes(length)
            else:
                data = bytes(machine.mem_read(arg, length))
            if length:
                machine.mem_write(dest, data)
            for register in fp[:16]:
                machine.reg_write(register, 0xDEADBEEF)
            machine.reg_write(reg.UC_ARM_REG_FPSCR, 0x03000000)
            for register in [reg.UC_ARM_REG_R0, reg.UC_ARM_REG_R1, reg.UC_ARM_REG_R2, reg.UC_ARM_REG_R3, reg.UC_ARM_REG_R12]:
                machine.reg_write(register, 0xBAD)
            called.append((address, machine.reg_read(reg.UC_ARM_REG_R4)))
            machine.reg_write(reg.UC_ARM_REG_PC, machine.reg_read(reg.UC_ARM_REG_LR))

        cpu.hook_add(UC_HOOK_CODE, firmware)
        cases = 0
        for operation in ["clr", "cpy"]:
            base = symbols["__wrap___aeabi_mem" + operation]
            assert all(symbols["__wrap___aeabi_mem" + operation + suffix] == base for suffix in ["4", "8"])
            for length in [0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 64, 65, 257]:
                # The shared general implementation supports arbitrary source
                # and destination alignment, including the aligned ABI variants.
                for alignment in range(8):
                    source, dest = 0x41000 + (7 - alignment), 0x42000 + alignment
                    pattern = bytes(i % 251 for i in range(300))
                    cpu.mem_write(0x41000, pattern)
                    cpu.mem_write(0x42000, b"\xA5" * 300)
                    cpu.reg_write(reg.UC_ARM_REG_SP, 0x38000)
                    cpu.reg_write(reg.UC_ARM_REG_LR, 0x18001)
                    for i, register in enumerate(fp):
                        cpu.reg_write(register, 0x3F000000 + i)
                    for i, register in enumerate(saved):
                        cpu.reg_write(register, 0x12340000 + i)
                    cpu.reg_write(reg.UC_ARM_REG_FPSCR, 0x60000000)
                    assert cpu.reg_read(reg.UC_ARM_REG_FPSCR) == 0x60000000, "emulator FPSCR initialization"
                    cpu.reg_write(reg.UC_ARM_REG_R0, dest)
                    cpu.reg_write(reg.UC_ARM_REG_R1, source if operation == "cpy" else length)
                    cpu.reg_write(reg.UC_ARM_REG_R2, length)
                    called.clear()
                    cpu.emu_start(base | 1, 0x18000, count=1000)
                    assert cpu.reg_read(reg.UC_ARM_REG_PC) == 0x18000
                    assert len(called) == 1
                    expected = bytearray(b"\xA5" * 300)
                    expected[alignment:alignment + length] = pattern[7-alignment:7-alignment+length] if operation == "cpy" else bytes(length)
                    assert bytes(cpu.mem_read(0x42000, 300)) == bytes(expected)
                    assert bytes(cpu.mem_read(0x41000, 300)) == pattern
                    assert cpu.reg_read(reg.UC_ARM_REG_SP) == 0x38000
                    assert cpu.reg_read(reg.UC_ARM_REG_FPSCR) == 0x60000000, (hex(cpu.reg_read(reg.UC_ARM_REG_FPSCR)), called)
                    assert [cpu.reg_read(r) for r in fp] == [0x3F000000+i for i in range(32)]
                    assert [cpu.reg_read(r) for r in saved] == [0x12340000+i for i in range(8)]
                    cases += 1
        print(f"Verified {cases} ARM memory-helper cases: bytes, alignment, stack, core/VFP registers and FPSCR.")


if __name__ == "__main__":
    main()
