"""Package a Rust staticlib for the Disting NT relocatable-object loader.

Firmware library imports must remain imports even when compiler_builtins contains
definitions with the same name. Wrap while selecting archive members, then
restore the firmware symbol name in the final object.
"""
import argparse
from pathlib import Path
import subprocess
from dram_callees import place


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--tool-prefix", default="arm-none-eabi-")
    parser.add_argument("--dram-callees", action="store_true",
                        help="Move direct callees used exclusively by explicit DRAM code")
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    imports = ["exp2f", "memset", "memcpy"]
    # Adapt the stronger runtime-helper register contract to ordinary C calls.
    # Unused helpers remain unselected in the archive.
    helpers = ["__aeabi_mem" + operation + alignment
               for operation in ["clr", "cpy"] for alignment in ["", "4", "8"]]
    subprocess.run([
        args.tool_prefix + "ld", "-r", "--gc-sections", "-u", "pluginEntry",
        *["--wrap=" + name for name in imports + helpers],
        "-o", str(args.output), str(args.archive),
    ], check=True)

    subprocess.run([
        args.tool_prefix + "objcopy", "--strip-debug",
        *["--redefine-sym=__wrap_" + name + "=" + name for name in imports],
        "--remove-section=.llvmbc", "--remove-section=.llvmcmd",
        "--remove-section=.ARM.exidx*", "--remove-section=.rel.ARM.exidx*",
        str(args.output),
    ], check=True)

    if args.dram_callees:
        place(args.output, args.tool_prefix)


if __name__ == "__main__":
    main()
