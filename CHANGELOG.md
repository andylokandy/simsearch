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

The easiest upgrade path is to fix the compile errors in this order:

1. Rename the public types and constructors.
2. Read search results from `Hit<Id>` instead of using IDs directly.
3. Replace token-based indexing and searching with `insert_parts(...)` and
   `search(...)`.
4. Move threshold filtering to caller code that filters by `Hit::score`.
5. Replace tokenizer options with `Options::separators(...)`.
6. Remove matching metric selection.
7. Review the new default result limit of 10.
8. Review ID bounds and tie-breaking.

#### 1. Rename types and constructors

`SimSearch` is now `Index`, `SearchOptions` is now `Options`, and
`SimSearch::new_with(...)` is now `Index::with_options(...)`.

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

#### 2. Read IDs and scores from hits

`search(...)` now returns `Vec<Hit<Id>>`. Each hit contains the matched `id`
and a normalized `score` in the `0.0..=1.0` range.

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

The old scored search APIs are folded into `search(...)`; every returned hit
includes both `id` and `score`.

#### 3. Replace token APIs with parts and queries

The old token APIs have two common migrations.

Use `insert_parts(...)` when your old `insert_tokens(...)` input represented
several searchable fields, aliases, or other document parts:

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

Use `search(...)` with a string query instead of `search_tokens(...)`. The same
built-in tokenizer is used for indexed parts and search queries.

There is no direct replacement for fully custom query tokenization. If you need
more control than separators provide, prepare searchable strings before
inserting them into the index and before passing queries to `search(...)`.

#### 4. Replace threshold filtering

`SearchOptions::threshold(...)` was removed. Search returns lower-quality
matches with lower scores, so callers can filter the returned hits.

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

#### 5. Replace tokenizer options with separators

`stop_whitespace(...)` and `stop_words(...)` were removed. Whitespace is always
a separator, and `Options::separators(...)` adds more separator characters.

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

If old `stop_words(...)` entries contained multi-character strings, or if you
used `stop_whitespace(false)`, there is no direct option replacement. Prepare
the searchable strings before inserting or searching.

#### 6. Remove matching metric selection

`SearchOptions::levenshtein(...)` was removed. The index uses Jaro-Winkler
similarity internally for typo-tolerant matching.

Before:

```rust
let options = SearchOptions::new().levenshtein(true);
```

After:

```rust
let options = Options::new();
```

Use `Options::typo_tolerance(false)` if you want to disable typo-tolerant
matching.

#### 7. Review result limits

Search now returns up to 10 results by default.

```rust
let options = Options::new().limit(20);
let mut index = Index::with_options(options);
```

#### 8. Review ID bounds and tie-breaking

IDs no longer need `Ord`. They need `Eq + Clone + Hash`, and equal-score ties
keep insertion order.
