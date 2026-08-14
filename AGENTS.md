# Repository Guidelines

## Project Structure & Module Organization

`m2-syn` is a Rust 2024 workspace for typed Macaulay2 syntax and future language analysis. The root crate lives in `src/`: `nodes.rs` is the compact syntax-graph source, `gen/` contains its checked-in generated expansion, `cst.rs` defines reconstruction interfaces, and `treesitter.rs` is the current parser adapter. Procedural macros and the generator live in `m2-syn-macros/src/`; integration tests live in `tests/`. Keep public concepts and examples in `README.md`, which is also crate documentation. Never hand-edit `src/gen/`.

## Current Scope & Architecture

This crate originated in the LSP but is now a standalone, `syn`-like foundation. The immediate milestone is a reliable round trip: read M2 source, parse it with Tree-sitter, reconstruct the complete typed AST, and emit normalized source that reparses equivalently. Prioritize broad round-trip correctness before expanding `quote_m2!`. Model syntax against the current grammar in the sibling `../tree-sitter-macaulay2` checkout; published `5.0.0` is substantially older until a new release is adopted.

Keep the typed graph parser-independent even though Tree-sitter is currently unconditional: parser details stop at `CstNode`/`treesitter.rs`. A future adapter may consume Macaulay2's built-in `Core` parser despite its different CST. Semantic/typechecking walks, incremental updates through changed references, node metadata, and M2 procedural macros are later roadmap work.

Prefer generation over boilerplate. Extend the smallest possible `syntax!` grammar description and derive construction, reconstruction, traversal, and printing mechanically where practical.

## Build, Test, and Development Commands

Run checks from the repository root:

```sh
cargo run -p m2-syn-macros --bin generate -- --check
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use `cargo run -p m2-syn-macros --bin generate` after changing the schema or generator, and commit the resulting `src/gen/` diff. Use `cargo fmt --all` to apply formatting. During iteration, target one suite with `cargo test --test generated_api` or a test-name filter.

## Coding Style & Naming Conventions

Follow `rustfmt`: four-space indentation and trailing commas in multiline constructs. Use `snake_case` for modules, functions, fields, and tests; `UpperCamelCase` for types and traits; and `SCREAMING_SNAKE_CASE` for constants. Keep generated APIs centralized rather than duplicating implementations.

## Testing Guidelines

Add focused integration tests in `tests/<feature>.rs` with behavior-oriented names, such as `fold_reconstructs_the_owned_tree`. Parser changes require real M2 examples and, where applicable, parse/emit/reparse assertions. Test generated APIs and errors at public boundaries; every regression should receive a narrow test.

## Commit & Pull Request Guidelines

History currently contains only `Initialize m2-syn workspace`; use concise, imperative subjects focused on one logical change. Pull requests should explain behavior and generated-code implications, list checks run, link issues, and include before/after M2 or typed-tree examples for syntax changes.
