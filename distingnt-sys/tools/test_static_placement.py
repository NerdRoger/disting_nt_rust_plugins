"""Verify initialization placement and that SysEx/string callbacks stay in ITC."""
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[2]
    with tempfile.TemporaryDirectory(prefix="nt-static-placement-") as directory:
        project = Path(directory)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text('''[package]
name = "static-placement-probe"
version = "0.1.0"
edition = "2021"
[workspace]
[lib]
crate-type = ["staticlib"]
[features]
dram = []
[dependencies]
distingnt-api = { path = "''' + (root / "distingnt-api").as_posix() + '''" }
[profile.plugin]
inherits = "release"
opt-level = "s"
panic = "abort"
codegen-units = 1
lto = true
''')
        (project / "src/lib.rs").write_text('''#![no_std]
#![forbid(unsafe_code)]
use distingnt_api::api::{plugin::*, state_memory::Sram};
use distingnt_api::{api::metadata::Definition, UNIT_STRING, SCALING_NONE};
struct Probe;
impl Plugin for Probe {
    type State = [u8; 8];
    type Voice = ();
    type StateMemory = Sram;
    const GUID: u32 = 1;
    const NAME: &'static core::ffi::CStr = c"Static probe";
    const DESCRIPTION: &'static core::ffi::CStr = c"";
    const TAGS: u32 = 0;
    const SPECIFICATIONS: &'static [Specification] = &[];
    const STATIC_INITIALIZATION: bool = true;
    const INITIALIZATION_IN_DRAM: bool = cfg!(feature = "dram");
    const MIDI_SYSEX: bool = true;
    const STRING_PARAMETERS: bool = true;
    fn calculate_static_requirements() -> StaticRequirements { StaticRequirements { dram: 16 } }
    fn initialise(memory: StaticMemory<'_>) { memory.dram.fill(42); }
    fn configuration(_: &[i32]) -> Option<Configuration> {
        Some(Configuration { parameters: 1, pages: 0, page_indices: 0, voices: 0 })
    }
    fn create(_: &[i32], _: u32) -> Option<Self::State> { Some([0; 8]) }
    fn create_voice(_: &[i32], _: usize) -> Option<()> { Some(()) }
    fn metadata(_: &[i32], metadata: &mut Metadata<'_>) -> Option<()> {
        metadata.set_parameter(0, Definition::new(c"Text", 0, 0, 0, UNIT_STRING, SCALING_NONE, None)?)
    }
    fn midi_sysex(message: &[u8]) {
        if message.first() == Some(&0xf0) { let _ = distingnt_api::api::sample_rate(); }
    }
    fn string_parameter_buffer<'a>(
        context: &'a mut Context<'_, Self::State, (), distingnt_api::api::parameters::ReadOnly>,
        _: i32,
    ) -> Option<&'a mut [u8]> { Some(context.state_mut()) }
}
distingnt_api::export_plugin!(Probe);
''')
        env = dict(os.environ, DISTINGNT_API_INCLUDE=str(root / "distingNT_API/include"))
        for dram in [False, True]:
            command = ["cargo", "build", "--manifest-path", str(project / "Cargo.toml"),
                       "--target", "thumbv7em-none-eabihf", "--profile", "plugin"]
            if dram:
                command += ["--features", "dram"]
            subprocess.run(command, cwd=root, env=env, check=True)
            archive = project / "target/thumbv7em-none-eabihf/plugin/libstatic_placement_probe.a"
            obj = project / ("dram.o" if dram else "itc.o")
            subprocess.run(["python3", str(root / "distingnt-sys/tools/link_plugin.py"),
                            str(archive), str(obj), "--dram-callees"], check=True)
            subprocess.run(["python3", str(root / "tools/check_plugin.py"), str(obj)], check=True)
            symbols = subprocess.check_output(["arm-none-eabi-objdump", "-tC", str(obj)], text=True)
            entries = [line for line in symbols.splitlines() if "Registration<" in line
                       and ("::calculate_static_requirements" in line or "::initialise" in line)]
            assert len(entries) == 2, entries
            assert all((".text.nt_dram" in line) == dram for line in entries), entries
            runtime = [line for line in symbols.splitlines() if "Registration<" in line
                       and ("::midi_sysex" in line or "::string_parameter_buffer" in line)]
            assert len(runtime) == 2, runtime
            assert all(".text.nt_dram" not in line for line in runtime), runtime
            print("Verified static initialization callbacks in " + ("DRAM" if dram else "ITC"))
            print("Verified SysEx and string-buffer callbacks remain in ITC")


if __name__ == "__main__":
    main()
