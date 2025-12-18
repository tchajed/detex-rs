# detex-rs

Port of [opendetex](https://github.com/pkubowicz/opendetex) to Rust using LLMs.

Claude Opus 4.5 did the majority of the work with this prompt, along with detex's `detex.l` (written in flex, a lexer generator, but largely C code) and `detex.h`:

> Attached is the source code for detex, written in flex and C. Port this to Rust. First make a careful plan to structure the code, then port everything. Try to write idiomatic Rust code.

## Building

```
cargo build
make -C opendetex-2.8.11 detex
```
