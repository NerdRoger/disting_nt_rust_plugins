"""Compile Rust and C++ ARM probes and compare their ABI metadata.

No hardware or emulation is needed: both probes emit the same metadata array
into an ELF section, which objcopy extracts for comparison.
"""
from pathlib import Path
import re
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[1]
INCLUDE = ROOT / "distingNT_API/include"
SRC = ROOT / "distingnt-sys/src"
OUT = ROOT / "target/api-check"
TARGET = "thumbv7em-none-eabihf"


def run(*args):
    subprocess.run([str(a) for a in args], cwd=ROOT, check=True)


def strip_comments(text):
    return re.sub(r"/\*.*?\*/|//[^\n]*", "", text, flags=re.S)


type_map = {
    "NtGlobals": "_NT_globals", "NtStaticRequirements": "_NT_staticRequirements",
    "NtStaticMemoryPtrs": "_NT_staticMemoryPtrs", "NtAlgorithmRequirements": "_NT_algorithmRequirements",
    "NtAlgorithmMemoryPtrs": "_NT_algorithmMemoryPtrs", "NtParameter": "_NT_parameter",
    "NtParameterPage": "_NT_parameterPage", "NtParameterPages": "_NT_parameterPages",
    "NtAlgorithm": "_NT_algorithm", "NtSpecification": "_NT_specification",
    "NtUiData": "_NT_uiData", "NtFactory": "_NT_factory", "NtSlot": "_NT_slot",
    "NtFileInfo": "_NT_fileInfo", "NtWavFolderInfo": "_NT_wavFolderInfo",
    "NtWavInfo": "_NT_wavInfo", "NtWavRequest": "_NT_wavRequest",
    "NtWavetableInfo": "_NT_wavetableInfo", "NtWavetableRequest": "_NT_wavetableRequest",
    "NtWavetableEvaluation": "_NT_wavetableEvaluation", "NtMidiFileFolderInfo": "_NT_midiFileFolderInfo",
    "NtStreamOpenData": "_NT_streamOpenData", "NtSclNote": "_NT_sclNote",
    "NtSclInfo": "_NT_sclInfo", "NtSclRequest": "_NT_sclRequest",
}


def camel(field):
    special = {"default": "def", "kind": "type", "midi_sysex": "midiSysEx",
               "mipmap_size": "mipmapSize", "using_mip_maps": "usingMipMaps"}
    if field in special:
        return special[field]
    first, *rest = field.split("_")
    return first + "".join(w.title() for w in rest)


def rust_type(cpp):
    cpp = re.sub(r"\bclass\s+", "", cpp.strip())
    if "(*" in cpp:
        return 'Option<unsafe extern "C" fn(*mut c_void)>'
    if cpp.endswith("&"):
        return ("*const " if cpp.startswith("const ") else "*mut ") + rust_type(cpp.removeprefix("const ")[:-1])
    if cpp.endswith("*"):
        return ("*const " if cpp.startswith("const ") else "*mut ") + rust_type(cpp.removeprefix("const ")[:-1])
    cpp = cpp.strip()
    base = {"void": "c_void", "char": "c_char", "int": "i32", "float": "f32", "bool": "bool",
            "uint8_t": "u8", "int8_t": "i8", "uint16_t": "u16", "int16_t": "i16",
            "uint32_t": "u32", "int32_t": "i32", "size_t": "usize", "uintptr_t": "usize",
            "_NT_fileHandle": "NtFileHandle", "_NT_fileResult": "NtFileResult",
            "_NT_stream": "NtStream", "_NT_frame": "NtFrame", "_NT_float3": "NtFloat3",
            "_NT_jsonStream": "NtJsonStream", "_NT_jsonParse": "NtJsonParse", "_NT_selector": "u8"}
    if cpp in base:
        return base[cpp]
    reverse = {v: k for k, v in type_map.items()}
    if cpp in reverse:
        return reverse[cpp]
    if cpp in {"_NT_textAlignment", "_NT_textSize", "_NT_shape", "_NT_findFlags"}:
        return "u8"
    raise ValueError(f"Unknown C++ ABI type: {cpp!r}")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    run("cargo", "build", "-p", "distingnt-sys", "--target", TARGET, "--release")
    rust = ["#![no_std]", "use core::ffi::{c_char,c_void};", "use distingnt_sys::*;",
            "use distingnt_sys::{file::*,wav::*,microtuning::*};"]
    cpp = ["#include <cstddef>", "#include <cstdint>"]
    headers = ["api.h", "file.h", "wav.h", "microtuning.h", "serialisation.h", "slot.h"]
    cpp += [f"#include <distingnt/{h}>" for h in headers]
    labels, rexprs, cexprs = [], [], []

    def check(label, r, c):
        labels.append(label)
        rexprs.append(f"({r}) as u32")
        cexprs.append(f"uint32_t({c})")

    sources = "\n".join(p.read_text() for p in SRC.glob("*.rs"))
    for name, ctype in type_map.items():
        check(name + ".size", f"core::mem::size_of::<{name}>()", f"sizeof({ctype})")
        check(name + ".align", f"core::mem::align_of::<{name}>()", f"alignof({ctype})")
        if name == "NtSlot":
            continue  # refCon is intentionally private in both languages.
        match = re.search(r"pub (?:struct|union) " + name + r"\s*\{([^}]+)\}", sources)
        assert match, name
        for field in re.findall(r"pub\s+(\w+)\s*:", match[1]):
            if name == "NtSclNote" and field == "ratio":
                continue  # anonymous struct in the C++ union
            check(name + "." + field, f"core::mem::offset_of!({name}, {field})", f"offsetof({ctype}, {camel(field)})")
    check("NtSclRatio.numerator", "core::mem::offset_of!(NtSclRatio, numerator_value)", "offsetof(_NT_sclNote, numeratorValue)")
    check("NtSclRatio.denominator", "core::mem::offset_of!(NtSclRatio, denominator_value)", "offsetof(_NT_sclNote, denominatorValue)")
    for enum in ["_NT_version", "_NT_selector", "_NT_textSize", "_NT_textAlignment", "_NT_shape", "_NT_midiDestination", "_NT_parameterUnit", "_NT_tag", "_NT_controls", "_NT_fileResult", "_NT_fileOpenMode", "_NT_findFlags", "_NT_fileAttrib", "_NT_wavProgress", "_NT_wavChannels", "_NT_wavBits", "_NT_roundRobinMode"]:
        expected = "u16" if enum == "_NT_controls" else "u8"
        check(enum + ".size", f"core::mem::size_of::<{expected}>()", f"sizeof({enum})")

    # Every uppercase Rust constant in the ABI modules must match an upstream
    # enumerator. Explicit spelling differences are listed here.
    constants = {
        "API_VERSION_CURRENT": "kNT_apiVersionCurrent", "LAST_BUS": "kNT_lastBus",
        "NUM_INPUT_BUSSES": "kNT_numInputBusses", "NUM_OUTPUT_BUSSES": "kNT_numOutputBusses", "NUM_AUX_BUSSES": "kNT_numAuxBusses",
        "MINIMUM_STEPS_IN_WORK_BUFFER": "kNT_minimumStepsInWorkBuffer",
        "PARAMETER_UI_PREFIX_SIZE": "kNT_parameterUiPrefixSize", "PARAMETER_STRING_SIZE": "kNT_parameterStringSize",
        "SCREEN_BYTES": "sizeof(NT_screen)", "NUM_RR_MODES": "kNT_numRRModes",
        "NUM_AVAILABLE_FILE_HANDLES": "kNT_numAvailableFileHandles",
        "FILE_OK": "kNT_fileOK", "DB_MIN_INF": "kNT_unitDb_minInf",
    }
    for version in range(1, 15):
        constants[f"API_VERSION_{version}"] = f"kNT_apiVersion{version}"
    for const, original in constants.items():
        if const != "DB_MIN_INF":
            check(const, const, original)
    header_text = "\n".join(strip_comments((INCLUDE / "distingnt" / h).read_text()) for h in headers)
    normalize = lambda value: value.replace("_", "").lower()
    upstream_constants = {normalize(v.removeprefix("kNT_")): v for v in re.findall(r"\bkNT_\w+", header_text)}
    for const in re.findall(r"pub const ([A-Z][A-Z_0-9]*):", sources):
        if const in constants or const == "EMPTY":
            continue
        original = upstream_constants.get(normalize(const.removeprefix("SHAPE_"))) if const.startswith("SHAPE_") else upstream_constants.get(normalize(const))
        if original is None:
            raise ValueError(f"No upstream enum equivalent found for {const}")
        check(const, const, original)

    signatures = 0
    def arguments(args):
        types = []
        for arg in args.split(","):
            arg = arg.strip().split("=")[0].strip()
            if arg in {"", "void"}:
                continue
            if "(*" in arg:
                types.append(rust_type(arg))
            else:
                types.append(rust_type(re.sub(r"\b\w+$", "", arg).strip()))
        return types
    for header in ["api.h", "file.h", "wav.h", "microtuning.h"]:
        text = strip_comments((INCLUDE / "distingnt" / header).read_text())
        for result, name, args in re.findall(r"^\s*([\w* ]+?)\s+(NT_\w+)\s*\((.*?)\)\s*;", text, flags=re.M | re.S):
            result = result.strip()
            types = arguments(args)
            ret = "" if result == "void" else " -> " + rust_type(result)
            rust.append(f'const _: unsafe extern "C" fn({", ".join(types)}){ret} = {name};')
            signatures += 1
    callback_count = 0
    api_text = strip_comments((INCLUDE / "distingnt/api.h").read_text())
    factory = api_text.split("struct _NT_factory", 1)[1].split('extern "C"', 1)[0]
    aliases = {camel(field): alias for field, alias in re.findall(r"pub\s+(\w+):\s*Option<(\w+)>", sources)}
    for result, name, args in re.findall(r"^\s*([\w* ]+?)\s*\(\s*\*\s*(\w+)\s*\)\s*\((.*?)\)\s*;", factory, flags=re.M | re.S):
        ret = "" if result.strip() == "void" else " -> " + rust_type(result)
        rust.append(f'const _: Option<unsafe extern "C" fn({", ".join(arguments(args))}){ret}> = None::<{aliases[name]}>;')
        callback_count += 1
    assert callback_count == 18, f"Expected every factory callback, got {callback_count}"
    rust += ['nt_dram_section! { #[no_mangle] pub extern "C" fn api_probe_dram(x: u32) -> u32 { x ^ 55 } }',
             f'#[used] #[unsafe(link_section=".nt_abi")] static ABI: [u32; {len(labels)}] = [', ",\n".join(rexprs), "];" ]
    cpp += ['extern "C" const uint32_t abi[] __attribute__((section(".nt_abi"))) = {', ",\n".join(cexprs), "};"]
    (OUT / "probe.rs").write_text("\n".join(rust))
    (OUT / "probe.cpp").write_text("\n".join(cpp))
    run("arm-none-eabi-c++", "-std=c++23", "-mcpu=cortex-m7", "-mfpu=fpv5-d16", "-mfloat-abi=hard", "-mthumb", "-I" + str(INCLUDE), "-c", OUT / "probe.cpp", "-o", OUT / "cpp.o")
    run("rustc", "--edition=2021", "--crate-type=lib", "--target", TARGET, "-Cpanic=abort", "--emit=obj", "--extern", f"distingnt_sys={ROOT}/target/{TARGET}/release/libdistingnt_sys.rlib", OUT / "probe.rs", "-o", OUT / "rust.o")
    for lang in ["cpp", "rust"]:
        run("arm-none-eabi-objcopy", "--dump-section", f".nt_abi={OUT}/{lang}.bin", OUT / f"{lang}.o")
    sections = subprocess.check_output(["arm-none-eabi-readelf", "-SW", str(OUT / "rust.o")], text=True)
    assert ".text.nt_dram" in sections, "DRAM function-placement macro did not create the requested section"
    cdata = struct.unpack("<" + "I" * len(labels), (OUT / "cpp.bin").read_bytes())
    rdata = struct.unpack("<" + "I" * len(labels), (OUT / "rust.bin").read_bytes())
    mismatches = [f"{label}: C++={c}, Rust={r}" for label, c, r in zip(labels, cdata, rdata) if c != r]
    if mismatches:
        raise SystemExit("\n".join(mismatches))
    print(f"API ABI verified: {len(labels)} ARM sizes/alignments/offsets/constants, {signatures} host functions, {callback_count} factory callbacks, DRAM section.")


if __name__ == "__main__":
    main()
