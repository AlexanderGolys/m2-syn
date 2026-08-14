# Generated code notes

`src/nodes.rs` is the source of truth. `src/gen/` is a checked-in build
artifact: review it, compile it, and never edit it by hand.

## Generate and verify

```sh
cargo run -p m2-syn-macros --bin generate
cargo run -p m2-syn-macros --bin generate -- --check
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Generate only after changing the schema or generator. `--check` should be the
normal CI and pre-commit path. Keep generation deterministic and test that a
second generation produces no diff.

## Keep expansions small

- Put shared behavior in handwritten generic types and functions; generate
  declarations, tables, match arms, and delegations into those primitives.
- Generate one central token spelling/kind table instead of repeating the same
  reconstruction and printing logic for every token, if the type-level API can
  be preserved.
- Feature-gate optional families such as `Visit`, `VisitMut`, and `Fold`, much
  like `syn` does. They are useful but need not be indexed in every session.
- Keep lexer, source positions, errors, and token-stream primitives
  handwritten. The generator may supply token names or expected-token tables,
  but should not generate the error machinery.
- Do not move the expansion to `OUT_DIR` merely to hide it. Rust Analyzer must
  still understand the expanded items, while review and source navigation get
  worse.
- Replacing checked-in files with one large procedural-macro expansion also
  does not remove the semantic work from Rust Analyzer; it can add proc-macro
  execution overhead.

## Make indexing switchable

A useful future feature split is:

```toml
[features]
default = ["ast", "visit", "visit-mut", "fold"]
ast = []
visit = ["ast"]
visit-mut = ["ast"]
fold = ["ast"]
```

Keep `span`, `error`, `lexer`, and the flat lexical token types outside `ast`.
Put `src/gen/{kind,tokens,nodes}.rs` behind `ast`, and gate each generated
traversal separately. This creates two honest development modes:

- full mode: default features, for AST/parser/reconstruction work;
- lexer mode: `--no-default-features`, so generated AST code is inactive.

Rust Analyzer can select the same modes with
`rust-analyzer.cargo.noDefaultFeatures` and `rust-analyzer.cargo.features`.
Reload the workspace after switching. Disabling cache priming can reduce
startup work, at the cost of moving some latency to the first query:

```json
{
  "rust-analyzer.cargo.noDefaultFeatures": true,
  "rust-analyzer.check.noDefaultFeatures": true,
  "rust-analyzer.cachePriming.enable": false
}
```

For generator-only work, `rust-analyzer.linkedProjects` can load only
`m2-syn-macros/Cargo.toml`. `rust-analyzer.procMacro.ignored` is useful only
for actual procedural-macro expansion; it does not help with the current
checked-in `include!` files. See the
[Rust Analyzer configuration reference](https://rust-analyzer.github.io/book/configuration.html).

## Search and agent context

Add `src/gen/` to a repository `.ignore` file to keep ordinary `rg` and editor
searches focused while leaving the tracked files in Git. Opt in with
`rg --no-ignore ... src/gen`. Also tell agents in `AGENTS.md` to inspect the
schema and generator first and open generated output only when validating the
exact emitted API or diff.

Codex does not need the whole expansion for normal reasoning. In the IDE,
avoid leaving generated files open because open files are automatically added
as context; add a generated file explicitly only for an expansion bug. See the
[Codex IDE context guidance](https://learn.chatgpt.com/docs/prompting).

## Tokenizer boundary

The tokenizer should produce flat, source-spanned lexical facts: identifiers,
number and string pieces, punctuation atoms, comments/trivia, physical line
breaks, and an end-of-input state. It should preserve joint versus separated punctuation and use
greedy, context-free matching. It must not decide precedence, synthesize
adjacency, or decide whether a newline terminates a cell; those are parser
decisions.

Use structured, spanned `LexError` and `ParseError` values. Lexer errors cover
invalid characters and unterminated lexical constructs; parser errors cover
unexpected tokens, missing operands, invalid constructs, and incomplete input.
Keep `ReconstructError` separate: it reports an adapter/schema mismatch, not
bad user syntax.
