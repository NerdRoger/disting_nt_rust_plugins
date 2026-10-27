"""Conservative, per-object placement of direct DRAM-only callees.

Only local executable sections reached by direct calls are eligible. Address
references (including factory callbacks), unknown callers and shared ITC callers
prevent movement. Existing explicit DRAM sections are the roots and stay put.
"""
import re
import subprocess
import tempfile
from pathlib import Path

CALLS = {"R_ARM_THM_CALL", "R_ARM_THM_JUMP24", "R_ARM_CALL", "R_ARM_JUMP24"}


def exclusive_callees(code, roots, edges):
    """edges contains (source section, target section, is_direct_call)."""
    reached = set(roots)
    while True:
        next_reached = reached | {target for source, target, direct in edges
                                 if direct and source in reached and target in code}
        if next_reached == reached:
            break
        reached = next_reached
    candidates = reached - roots
    # Eliminate shared/address-exposed nodes to a fixed point. This also handles
    # mutually recursive groups reached only from DRAM, without splitting them.
    while True:
        rejected = {target for source, target, direct in edges
                    if target in candidates and
                    (not direct or source not in candidates | roots)}
        if not rejected:
            return candidates
        candidates -= rejected


def inspect(path, prefix):
    def output(command):
        return subprocess.check_output([prefix + "readelf", command, str(path)], text=True)
    sections = {}
    for line in output("-SW").splitlines():
        match = re.match(r"\s*\[\s*(\d+)\]\s+(\S+)\s+\S+\s+\S+\s+\S+\s+\S+\s+\S+\s+(\S+)", line)
        if match:
            sections[int(match[1])] = (match[2], match[3])
    symbols = {}
    functions = set()
    for line in output("-sW").splitlines():
        match = re.match(r"\s*(\d+):\s+\S+\s+\S+\s+(\S+)\s+\S+\s+\S+\s+(\S+)\s*(.*)", line)
        if match and match[3].isdigit():
            section = int(match[3])
            symbols[int(match[1])] = section
            if match[2] == "FUNC":
                functions.add(section)
    by_name = {name: index for index, (name, flags) in sections.items()}
    edges = []
    source = None
    for line in output("-rW").splitlines():
        header = re.match(r"Relocation section '([^']+)'", line)
        if header:
            # ARM objects are ELF32/REL; also recognize RELA section names.
            target = re.sub(r"^\.rela?", "", header[1])
            source = by_name.get(target)
            continue
        relocation = re.match(r"\s*[0-9a-fA-F]+\s+([0-9a-fA-F]+)\s+(R_ARM_\w+)", line)
        if source is not None and relocation:
            target = symbols.get(int(relocation[1], 16) >> 8)
            if target is not None:
                edges.append((source, target, relocation[2] in CALLS))
    code = {index for index, (_, flags) in sections.items() if "X" in flags and index in functions}
    roots = {index for index in code if sections[index][0] == ".text.nt_dram"
             or sections[index][0].startswith(".text.nt_dram.")}
    return sections, exclusive_callees(code, roots, edges)


def place(path, prefix):
    sections, selected = inspect(path, prefix)
    if not selected:
        return
    names = sorted(sections[index][0] for index in selected)
    # Rename individual function sections, then let the relocatable linker merge
    # them into one canonical DRAM section, updating symbols and relocations.
    with tempfile.TemporaryDirectory(prefix="nt-dram-callees-") as directory:
        renamed = Path(directory) / "placed.o"
        subprocess.run([prefix + "objcopy",
                        *["--rename-section=" + name + "=.text.nt_dram" for name in names],
                        str(path), str(renamed)], check=True)
        subprocess.run([prefix + "ld", "-r", "-o", str(path), str(renamed)], check=True)
    remaining_sections, remaining = inspect(path, prefix)
    if remaining:
        raise RuntimeError("Incomplete DRAM callee placement: " +
                           repr([remaining_sections[index][0] for index in remaining]))
    print("DRAM-only callees: " + ", ".join(names))
