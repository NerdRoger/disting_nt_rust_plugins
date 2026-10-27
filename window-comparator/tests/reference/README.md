# C++ reference fixture

Snapshot copied from NerdRoger/disting_nt_plugins, base commit `7319f0910d2911118e03e4719769df02bc2c0f3a`,
on 2026-10-27. The source working tree, including uncommitted Rust work, was the
migration source. These C++ files are test fixtures only: the native comparison
harness compiles them under sanitizers and compares its trace with Rust.
They are never linked into the Rust plugin. Root LICENSE applies.

The original C++ plugin remains maintained in the separate C++ repository.
This fixture has no runtime or filesystem dependency on that repository.
