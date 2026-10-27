"""Reject compiler bitcode, unresolved dependencies, and unsupported relocations."""
import re
import subprocess
import sys


def output(*args):
    return subprocess.check_output(args, text=True)


path = sys.argv[1]
sections = output("arm-none-eabi-readelf", "-SW", path)
compiler_sections = re.findall(r"\]\s+(\.llvm(?:bc|cmd))\s", sections)
if compiler_sections:
    sys.exit(f"Invalid plugin {path}: embedded compiler sections={compiler_sections}")
allowed_relocations = {
    "R_ARM_ABS32", "R_ARM_REL32", "R_ARM_BASE_PREL", "R_ARM_GOT_BREL",
    "R_ARM_THM_CALL", "R_ARM_THM_JUMP24",
}
allowed_cpp = {
    "_ZN13_NT_jsonParse10skipMemberEv",
    "_ZN13_NT_jsonParse21numberOfObjectMembersERi",
    "_ZN13_NT_jsonParse7booleanERb",
    "_ZN13_NT_jsonParse9matchNameEPKc",
    "_ZN14_NT_jsonStream10addBooleanEb",
    "_ZN14_NT_jsonStream13addMemberNameEPKc",
}
undefined = [line.split()[-1] for line in output("arm-none-eabi-nm", "-u", path).splitlines()]
def is_host_cpp(symbol):
    return symbol in allowed_cpp or re.match(r"^_ZNK?\d+_NT_(?:jsonStream|jsonParse|slot)\d+", symbol) is not None


unexpected = [s for s in undefined if not (s.startswith("NT_") or s in {"_GLOBAL_OFFSET_TABLE_", "exp2f", "memset", "memcpy"} or is_host_cpp(s))]
relocations = set(re.findall(r"\bR_ARM_\w+", output("arm-none-eabi-readelf", "-rW", path)))
unsupported = relocations - allowed_relocations
if unexpected or unsupported:
    sys.exit(f"Invalid plugin {path}: unresolved={unexpected}, unsupported relocations={sorted(unsupported)}")
if not re.search(r"\bT pluginEntry$", output("arm-none-eabi-nm", path), re.MULTILINE):
    sys.exit(f"Missing pluginEntry in {path}")
print(f"Verified {path}: resolved Rust dependencies; {len(relocations)} supported relocation forms.")
