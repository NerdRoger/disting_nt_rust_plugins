OUTPUT := plugins/rusttest.o
RUST_TARGET := thumbv7em-none-eabihf
API_SOURCES := $(wildcard distingnt-api/src/*.rs distingnt-api/src/api/*.rs distingnt-sys/src/*.rs distingnt-sys/shims/*.cpp distingnt-sys/tools/*.py distingNT_API/include/distingnt/*.h) distingnt-api/Cargo.toml distingnt-sys/Cargo.toml
.DELETE_ON_ERROR:

all: $(OUTPUT) plugins/rust-window-comparator.o plugins/wavesmith.o

plugins/wavesmith.o: Makefile tools/check_plugin.py Cargo.toml .cargo/config.toml wavesmith/Cargo.toml $(wildcard wavesmith/src/*.rs) distingnt-sys/build.rs $(API_SOURCES)
	@mkdir -p $(dir $@)
	cargo build --manifest-path Cargo.toml -p wavesmith --target $(RUST_TARGET) --profile plugin
	python3 distingnt-sys/tools/link_plugin.py target/$(RUST_TARGET)/plugin/libwavesmith.a $@ --dram-callees
	python3 tools/check_plugin.py $@

# Always rebuild this opt-in variant; normal/release artifacts remain separate.
wavesmith-profile:
	@mkdir -p plugins
	cargo build --manifest-path Cargo.toml -p wavesmith --target $(RUST_TARGET) --profile plugin --features profiling
	python3 distingnt-sys/tools/link_plugin.py target/$(RUST_TARGET)/plugin/libwavesmith.a plugins/wavesmith-profile.o --dram-callees
	python3 tools/check_plugin.py plugins/wavesmith-profile.o

.PHONY: wavesmith-profile

plugins/rust-window-comparator.o: Makefile tools/check_plugin.py Cargo.toml .cargo/config.toml window-comparator/Cargo.toml $(wildcard window-comparator/src/*.rs) distingnt-sys/build.rs $(API_SOURCES)
	@mkdir -p $(dir $@)
	cargo build --manifest-path Cargo.toml -p window-comparator --target $(RUST_TARGET) --profile plugin
	python3 distingnt-sys/tools/link_plugin.py target/$(RUST_TARGET)/plugin/libwindow_comparator.a $@ --dram-callees
	python3 tools/check_plugin.py $@

$(OUTPUT): Makefile tools/check_plugin.py Cargo.toml .cargo/config.toml distingnt-api/Cargo.toml distingnt-sys/build.rs $(API_SOURCES) monosynth/Cargo.toml $(wildcard monosynth/src/*.rs)
	@mkdir -p $(dir $@)
	cargo build --manifest-path Cargo.toml -p monosynth --target $(RUST_TARGET) --profile plugin
	python3 distingnt-sys/tools/link_plugin.py target/$(RUST_TARGET)/plugin/libmonosynth.a $@
	python3 tools/check_plugin.py $@

clean:
	rm -f $(OUTPUT) plugins/rust-window-comparator.o plugins/wavesmith.o plugins/wavesmith-profile.o
	rm -rf target

.PHONY: all clean

test:
	@mkdir -p target
	python3 tools/check_layers.py
	python3 distingnt-sys/tools/test_dram_callees.py
	python3 tools/check_api.py
	python3 -m unittest discover -s wavesmith/tools -p test_monitor_cycles.py
	python3 distingnt-sys/tools/test_static_placement.py
	g++ -g -fsanitize=address,undefined -std=gnu++23 -fpermissive -IdistingNT_API/include -Iwindow-comparator/tests/reference/Common/include -Iwindow-comparator/tests/reference/WindowComparator/include window-comparator/tests/cpp_reference.cpp -o target/window-cpp-reference
	./target/window-cpp-reference > target/window-cpp-reference.txt
	cargo test -p window-comparator --lib -- --include-ignored
	cargo test -p monosynth --lib
	cargo test -p wavesmith --lib
	cargo test -p wavesmith --lib --features profiling
	cargo test -p distingnt-sys --lib
	cargo test -p distingnt-api --lib
	cargo test -p distingnt-api --doc

.PHONY: test
