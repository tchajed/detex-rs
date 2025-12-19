# detex-rs: port of opendetex to rust

This is a port of opendetex (a tool for converting latex to plain text) to Rust.

The goals are compatibility and correctness.

The source code for opendetex is provided in opendetex-2.8.11 as a reference. Always read opendetex-2.8.11/detex.l before doing any code edits in the rust code. Do not modify anything in opendetex.

Use `gcat -A` to inspect files. Do not use `od`.
