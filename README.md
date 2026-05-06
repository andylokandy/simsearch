# `simsearch`

[![Continuous integration](https://github.com/andylokandy/simsearch-rs/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/andylokandy/simsearch-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/simsearch.svg)](https://crates.io/crates/simsearch)
[![docs.rs](https://docs.rs/simsearch/badge.svg)](https://docs.rs/simsearch)
[![MSRV 1.85.0](https://img.shields.io/badge/MSRV-1.85.0-green?style=flat-square&logo=rust)](https://www.whatrustisit.com)

A small in-memory fuzzy search index for embedded autocomplete and search
suggestions.

### [**Documentation**](https://docs.rs/simsearch)

## Usage

Add the following to your `Cargo.toml`:

```toml
[dependencies]
simsearch = "0.4"
```

## Example

```rust
use simsearch::Index;

let mut engine: Index<u32> = Index::new();

engine.insert(1, "Things Fall Apart");
engine.insert(2, "The Old Man and the Sea");
engine.insert(3, "James Joyce");

let results = engine.search("thngs");

assert_eq!(results[0].id, 1);
assert!(results[0].score > 0.0);
```

You can also try the interactive demo:

```sh
cargo run --release --example books
```

## Contributing

Contributions are welcome.

- **Issues.** Open an issue if you find a typo, hit a bug, or have a question.
- **Pull requests.** New collections, implementation improvements, tests, documentation, and typo fixes are welcome.

## License

Licensed under the MIT license ([LICENSE](LICENSE) or http://opensource.org/licenses/MIT).
