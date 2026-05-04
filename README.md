# `simsearch`

[![Build Status](https://travis-ci.com/andylokandy/simsearch-rs.svg?branch=master)](https://travis-ci.com/andylokandy/simsearch-rs)
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

Search returns up to 10 results by default. It supports last-token prefix
matching and typo tolerance by default, which fits search boxes and autocomplete
suggestions.

Also try the interactive demo by:

```
$ cargo run --release --example books
```

## Contribution

All kinds of contribution are welcomed.

- **Issues.** Feel free to open an issue when you find typos, bugs, or have any question.
- **Pull requests**. New collection, better implementation, more tests, more documents and typo fixes are all welcomed.

## License

Licensed under MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
