# Development

Run workspace checks from the repository root:

```sh
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Also check the native-only configuration when changing code shared by the parser
backends:

```sh
cargo check --workspace --no-default-features
```

The schema expands during compilation. A change to `src/schema/` is
picked up by the next build, without a generated-source update.

## Building this book

Install mdBook and the highlighting preprocessor from their published packages:

```sh
cargo install mdbook --locked
cargo install mdbook-tsitter --version 3.3.0 --locked
mdbook build book
```

For a local preview:

```sh
mdbook serve book --open
```

The book uses `mdbook-tsitter` for Rust, shell, and Macaulay2 code. Language
entries in `book/book.toml` select grammars from the preprocessor's built-in
list; the `sh` and `m2` aliases select Bash and Macaulay2 respectively. The
preprocessor fetches and compiles those grammars on the first build, which needs
network access, Git, and a C compiler. Later builds reuse its user cache.

The bundled highlighting stylesheet follows mdBook's light and dark themes.
No platform-specific parser library or copied query file is needed in the book.
Use a `text` fence for diagrams and a language fence for code. Schema excerpts
that illustrate declarations rather than runnable programs use `rust,ignore`.
Playground execution is disabled because the examples depend on `m2-syn`;
run them in a local project with that dependency.

## Examples and licensing

`cargo run --example inspect -- path/to/file.m2` displays source, tokens, and
the typed tree. The `hierarchy_visit_emit` and `type_inference` examples show
larger traversals.

`m2-syn` is dual-licensed under
[MIT](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE-MIT) or
[Apache-2.0](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE-APACHE),
at your option.
