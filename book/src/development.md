# Development

Run the complete workspace checks from the repository root:

```sh
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The schema is expanded by the `syntax!` procedural macro, so changes to
`src/schema/nodes.rs` are picked up by the next compile; there is no generated source
to refresh or commit.

`m2-syn` is licensed under the [GNU General Public License v3.0](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE).
