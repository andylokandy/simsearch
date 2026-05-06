# Changelog

## 0.4.0

This release redesigns `simsearch` for embedded autocomplete and search
suggestions. The public API is smaller, searches return scored hits, and entries
can be indexed from multiple parts without custom tokenization.

### Breaking Changes

- Renamed `SimSearch` to `Index`.
- Renamed `SearchOptions` to `Options`.
- Renamed `SimSearch::new_with(...)` to `Index::with_options(...)`.
- Changed `Index::search(...)` to return `Vec<Hit<Id>>` instead of `Vec<Id>`.
- Removed `insert_tokens(...)`, `search_tokens(...)`,
  `search_with_scores(...)`, and `search_tokens_with_scores(...)`.
- Added `insert_parts(...)` for entries with multiple searchable parts.
- Removed `SearchOptions::threshold(...)`. Search results are no longer filtered
  by a threshold; callers can filter by `Hit::score` when needed.
- Removed configurable matching metrics and `SearchOptions::levenshtein(...)`.
  Typo tolerance now uses Jaro-Winkler similarity internally.
- Replaced tokenizer options with `Options::separators(...)`, which adds
  separators beyond the default
  [`char::is_whitespace`](https://doc.rust-lang.org/std/primitive.char.html#method.is_whitespace)
  characters.
- Added a default result limit of 10. Use `Options::limit(...)` to change it.
- Removed the `Ord` requirement for IDs. IDs now need `Eq + Clone + Hash`;
  equal-score ties are resolved by insertion order.
- Bumped the crate to Rust 2024 edition and set MSRV to Rust 1.85.

### Added

- `Hit<Id>` with `id` and normalized `score`.
- `Index::insert_parts(id, parts)` for indexing multiple searchable parts.
- `Options::limit(...)` for controlling result count.
- `Options::prefix_search(...)` for controlling last-token prefix matching.
- `Options::typo_tolerance(...)` for controlling typo-tolerant matching.
- Scores in the interactive `books` example.

### Changed

- Search now uses a positional inverted index with exact, last-token prefix,
  typo-tolerant prefix, and Jaro-Winkler typo-tolerant term expansion.
- Search results are ranked by a single normalized relevance score in the
  `0.0..=1.0` range.
- Low-quality matches are allowed, but they receive lower scores and rank behind
  stronger matches.
- `insert(...)`, `insert_parts(...)`, and `search(...)` all use the built-in
  tokenizer.

### Migration Guide

#### Rename the main types

Before:

```rust
use simsearch::{SearchOptions, SimSearch};

let options = SearchOptions::new();
let mut index: SimSearch<u32> = SimSearch::new_with(options);
```

After:

```rust
use simsearch::{Index, Options};

let options = Options::new();
let mut index: Index<u32> = Index::with_options(options);
```

#### Read IDs from search hits

`search(...)` now returns `Vec<Hit<Id>>`.

Before:

```rust
let results: Vec<u32> = index.search("old sea");
let first_id = results[0];
```

After:

```rust
let results = index.search("old sea");
let first_id = results[0].id;
let first_score = results[0].score;
```

If you only need IDs, map the hits:

```rust
let ids: Vec<u32> = index
    .search("old sea")
    .into_iter()
    .map(|hit| hit.id)
    .collect();
```

The old scored search APIs are also replaced by `search(...)`; every returned
hit includes both `id` and `score`.

#### Replace token APIs with parts

The old token APIs were often used to avoid manually concatenating several
document parts. Use `insert_parts(...)` for that case.

Before:

```rust
index.insert_tokens(1, &["The Old Man and the Sea", "Ernest Hemingway"]);
let results = index.search_tokens(&["hemingway"]);
```

After:

```rust
index.insert_parts(1, ["The Old Man and the Sea", "Ernest Hemingway"]);
let results = index.search("hemingway");
```

There is no direct replacement for fully custom query tokenization. The index
now intentionally uses one built-in tokenizer for both indexed parts and search
queries.

#### Replace threshold filtering

Before:

```rust
let options = SearchOptions::new().threshold(0.8);
```

After:

```rust
let results: Vec<_> = index
    .search("old sea")
    .into_iter()
    .filter(|hit| hit.score >= 0.8)
    .collect();
```

Search applies `Options::limit(...)` before caller-side filtering. Increase the
limit first if you need more results before filtering by score.

#### Update tokenizer options

Before:

```rust
let options = SearchOptions::new()
    .stop_whitespace(true)
    .stop_words(vec!["/".to_string()]);
```

After:

```rust
let options = Options::new().separators(['/']);
```

`separators(...)` adds separators on top of the default
`char::is_whitespace` behavior.

#### Update result limits

Search now returns up to 10 results by default.

```rust
let options = Options::new().limit(20);
let mut index = Index::with_options(options);
```
