# Development

Run the complete workspace checks from the repository root:

```sh
cargo run -p m2-syn-macros --bin generate -- --check
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

After changing `src/nodes.rs` or the generator, refresh the expansion with
`cargo run -p m2-syn-macros --bin generate` and include the resulting
`src/gen/` diff.

`m2-syn` is licensed under the [GNU General Public License v3.0](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE).
