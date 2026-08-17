# Repository Guidelines

## Project Structure & Module Organization

`m2-syn` is a Rust workspace for typed Macaulay2 syntax tree and parsing tools. The package contains the language processing toolkit inspired with 
Rusts' `syn`, `quote` and `proc_macro` crates, with the grammar tree being based on `tree-sitter-macaulay2` that is also available as one of two parsing 
API's, the second one being implemented natively in this crate, see the the `parse` module. Foundational goals that the code in `m2-syn` was primarily
created for are 
- the interface for conveniently dispatched graph walks that can be achieved by implementing in the typed AST graph on chosen nodes
overriding the default implementation of Visit, VisitMut and Fold traits
- Reliable and fully correct Macaulay2 lexer and parser 
- Typed representation of tree-sitter grammar 
- A general basic engine for declarative and procedural macros
- Easy to use pattern matching syntax that makes writing macro close to native language 

## Current Scope & Architecture
- tba

## Build, Test, and Development Commands

Run checks from the repository root:

```sh
cargo run -p m2-syn-macros --bin generate -- --check
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use `cargo run -p m2-syn-macros --bin generate` after changing the schema or generator

## Coding Style & Naming Conventions
- Always pay special attention to checking whether there is no similar already implemented logicf or types 
- Try to preserve the package cross-module type system, keep the extensions minimal and only introduce unrelated new types when necessary 
- Prefer starting from the possibly general abstraction and specialising the code to the specific case needed at later stageyy 
- Document in detail every part of public API
- Add a few focused unit tests on the bottom of source files
- Tests should always use understandable input format and give back formatted result
- Don't document the particular implementations, functions, associated items or types, the code should be self-documenting enough for these to have clear meaning
- Always document new types, traits and modules 
- Whenever possible include examples in documentation
- Try to follow the cargo packaging guidlines for documentation, naming conventions and any suggested low-cost improvements

## Testing Guidelines

Add focused integration tests in `tests/<feature>.rs` with behavior-oriented names, such as `fold_reconstructs_the_owned_tree`. Parser changes require real M2 examples and, where applicable, parse/emit/reparse assertions. Test generated APIs and errors at public boundaries; every regression should receive a narrow test.

## Commit & Pull Request Guidelines

History currently contains only `Initialize m2-syn workspace`; use concise, imperative subjects focused on one logical change. Pull requests should explain behavior and generated-code implications, list checks run, link issues, and include before/after M2 or typed-tree examples for syntax changes.
