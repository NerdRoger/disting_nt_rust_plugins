"""Keep production plugins on the safe facade; raw access is test-only."""
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parents[1]
for plugin in ("monosynth", "window-comparator"):
    manifest = tomllib.loads((root / plugin / "Cargo.toml").read_text())
    dependencies = manifest["dependencies"]
    assert "distingnt-api" in dependencies, plugin
    assert "distingnt-sys" not in dependencies, plugin
    assert "host-test" not in dependencies["distingnt-api"].get("features", []), plugin
api = tomllib.loads((root / "distingnt-api/Cargo.toml").read_text())
assert "distingnt-sys" in api["dependencies"]
assert not (root / "distingnt-api/build.rs").exists()
assert not (root / "distingnt-api/shims").exists()
assert (root / "distingnt-sys/build.rs").exists()
print("Verified layers: plugins -> safe API -> sys; raw access is dev-only.")
