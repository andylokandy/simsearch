//! A small in-memory fuzzy search index for embedded autocomplete and search
//! suggestions.
//!
//! # Examples
//!
//! ```
//! use simsearch::Index;
//!
//! let mut engine: Index<u32> = Index::new();
//!
//! engine.insert(1, "Things Fall Apart");
//! engine.insert(2, "The Old Man and the Sea");
//! engine.insert(3, "James Joyce");
//!
//! let results = engine.search("thngs");
//!
//! assert_eq!(results[0].id, 1);
//! ```

use std::cmp::{Ordering, max};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use strsim::jaro_winkler;

const QUALITY_WEIGHT: f64 = 0.68;
const COVERAGE_WEIGHT: f64 = 0.14;
const PROXIMITY_WEIGHT: f64 = 0.07;
const EXACTNESS_WEIGHT: f64 = 0.04;
const POSITION_WEIGHT: f64 = 0.03;
const SPECIFICITY_WEIGHT: f64 = 0.04;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// An in-memory fuzzy search index.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Index<Id>
where
    Id: Eq + Clone + Hash,
{
    options: Options,
    id_num_counter: usize,
    ids_map: HashMap<Id, usize>,
    reverse_ids_map: HashMap<usize, Id>,
    forward_map: HashMap<usize, Vec<String>>,
    reverse_map: HashMap<String, Vec<usize>>,
}

/// A search result with its normalized relevance score.
///
/// Scores range from `0.0` to `1.0` and are meaningful for comparing results
/// from the same search query. Results are sorted by this score, with insertion
/// order used as the final tie-breaker.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Hit<Id> {
    /// The id associated with the matched entry.
    pub id: Id,
    /// A normalized relevance score in the `0.0..=1.0` range.
    pub score: f64,
}

#[derive(Debug, Clone)]
struct RankedResult<Id> {
    id_num: usize,
    id: Id,
    rank: Rank,
}

#[derive(Debug, Clone)]
struct Rank {
    score: f64,
}

#[derive(Debug, Clone)]
struct TokenMatch {
    query_index: usize,
    doc_index: usize,
    score: f64,
    typo_cost: usize,
    exact: bool,
}

#[derive(Debug, Clone, Copy)]
struct TokenSimilarity {
    score: f64,
    typo_cost: usize,
    exact: bool,
}

impl Rank {
    fn from_matches(matches: &[TokenMatch], query_len: usize, doc_len: usize) -> Self {
        let matched_terms = matches.len();
        let proximity_cost = Self::proximity_cost(matches, doc_len);
        let first_position = matches
            .iter()
            .map(|token_match| token_match.doc_index)
            .min()
            .unwrap_or(usize::MAX);
        let exact_terms = matches
            .iter()
            .filter(|token_match| token_match.exact)
            .count();
        let quality = matches
            .iter()
            .map(|token_match| token_match.score)
            .sum::<f64>()
            / query_len as f64;

        let coverage = matched_terms as f64 / query_len as f64;
        let proximity_score = if matched_terms < 2 {
            coverage
        } else {
            1.0 / (1.0 + proximity_cost as f64 / (matched_terms - 1) as f64)
        };
        let exactness = exact_terms as f64 / query_len as f64;
        let position_score = Self::position_score(first_position, doc_len);
        let specificity = matched_terms as f64 / doc_len as f64;
        let weighted_bonus = QUALITY_WEIGHT
            + COVERAGE_WEIGHT * coverage
            + PROXIMITY_WEIGHT * proximity_score
            + EXACTNESS_WEIGHT * exactness
            + POSITION_WEIGHT * position_score
            + SPECIFICITY_WEIGHT * specificity;
        let score = quality * weighted_bonus;

        Rank {
            score: score.clamp(0.0, 1.0),
        }
    }

    fn proximity_cost(matches: &[TokenMatch], doc_len: usize) -> usize {
        if matches.len() < 2 {
            return 0;
        }

        let mut cost = 0;
        for window in matches.windows(2) {
            let lhs = window[0].doc_index;
            let rhs = window[1].doc_index;
            if rhs > lhs {
                cost += rhs - lhs - 1;
            } else {
                cost += doc_len + lhs - rhs + 1;
            }
        }
        cost
    }

    fn position_score(first_position: usize, doc_len: usize) -> f64 {
        if first_position >= doc_len {
            return 0.0;
        }

        if doc_len <= 1 {
            return 1.0;
        }

        1.0 - first_position as f64 / (doc_len - 1) as f64
    }
}

impl<Id> Index<Id>
where
    Id: Eq + Clone + Hash,
{
    /// Creates an index with default options.
    pub fn new() -> Self {
        Self::with_options(Options::new())
    }

    /// Creates an index with custom options.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::{Options, Index};
    ///
    /// let mut engine: Index<usize> = Index::with_options(
    ///     Options::new().case_sensitive(true));
    /// ```
    pub fn with_options(options: Options) -> Self {
        Index {
            options,
            id_num_counter: 0,
            ids_map: HashMap::new(),
            reverse_ids_map: HashMap::new(),
            forward_map: HashMap::new(),
            reverse_map: HashMap::new(),
        }
    }

    /// Inserts an entry into the index.
    ///
    /// Input will be tokenized according to the search option.
    /// By default whitespaces(including tabs) are considered as separators,
    /// you can change the behavior by providing `Options`.
    ///
    /// Insert with an existing id updates the content.
    ///
    /// **Note that** id is not searchable. Add id to the contents if you would
    /// like to perform search on it.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::{Options, Index};
    ///
    /// let mut engine: Index<&str> = Index::with_options(
    ///     Options::new().separators(vec![",".to_string(), ".".to_string()]));
    ///
    /// engine.insert("BoJack Horseman", "BoJack Horseman, an American
    /// adult animated comedy-drama series created by Raphael Bob-Waksberg.
    /// The series stars Will Arnett as the title character,
    /// with a supporting cast including Amy Sedaris,
    /// Alison Brie, Paul F. Tompkins, and Aaron Paul.");
    /// ```
    pub fn insert(&mut self, id: Id, content: &str) {
        let tokens = self.tokenize(&[content]);
        self.insert_normalized_tokens(id, tokens)
    }

    /// Inserts an entry with multiple searchable fields into the index.
    ///
    /// Each field is tokenized with the same built-in tokenizer used by
    /// [`Index::insert`]. This is useful when an item has several searchable
    /// fields, such as a title, author, tags, or aliases.
    ///
    /// Insert with an existing id updates the content.
    ///
    /// **Note that** id is not searchable. Add id to the contents if you would
    /// like to perform search on it.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::Index;
    ///
    /// let mut engine: Index<&str> = Index::new();
    ///
    /// engine.insert_many("A Game of Thrones", [
    ///     "A Game of Thrones",
    ///     "George R. R. Martin",
    ///     "fantasy",
    /// ]);
    ///
    /// let results = engine.search("martin");
    ///
    /// assert_eq!(results[0].id, "A Game of Thrones");
    /// ```
    pub fn insert_many<I, S>(&mut self, id: Id, fields: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let fields: Vec<String> = fields
            .into_iter()
            .map(|field| field.as_ref().to_string())
            .collect();
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        let tokens = self.tokenize(&fields);
        self.insert_normalized_tokens(id, tokens)
    }

    fn insert_normalized_tokens(&mut self, id: Id, tokens: Vec<String>) {
        self.remove(&id);

        let id_num = self.id_num_counter;
        self.ids_map.insert(id.clone(), id_num);
        self.reverse_ids_map.insert(id_num, id);
        self.id_num_counter += 1;

        let mut indexed_tokens = HashSet::new();
        for token in tokens.clone() {
            if indexed_tokens.insert(token.clone()) {
                self.reverse_map
                    .entry(token)
                    .or_insert_with(|| Vec::with_capacity(1))
                    .push(id_num);
            }
        }

        self.forward_map.insert(id_num, tokens);
    }

    /// Searches pattern and returns hits sorted by relevance.
    ///
    /// Pattern will be tokenized according to the search option.
    /// By default whitespaces(including tabs) are considered as separators,
    /// you can change the behavior by providing `Options`.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::Index;
    ///
    /// let mut engine: Index<u32> = Index::new();
    ///
    /// engine.insert(1, "Things Fall Apart");
    /// engine.insert(2, "The Old Man and the Sea");
    /// engine.insert(3, "James Joyce");
    ///
    /// let results = engine.search("thngs apa");
    ///
    /// assert_eq!(results[0].id, 1);
    /// assert!(results[0].score > 0.0);
    /// ```
    pub fn search(&self, pattern: &str) -> Vec<Hit<Id>> {
        self.search_ranked(self.tokenize(&[pattern]))
            .into_iter()
            .map(|result| Hit {
                id: result.id,
                score: result.rank.score,
            })
            .collect()
    }

    fn search_ranked(&self, pattern_tokens: Vec<String>) -> Vec<RankedResult<Id>> {
        if pattern_tokens.is_empty() || self.options.limit == 0 {
            return Vec::new();
        }

        let candidates = self.collect_candidates(&pattern_tokens);
        let mut results: Vec<RankedResult<Id>> = candidates
            .into_iter()
            .filter_map(|id_num| {
                let tokens = self.forward_map.get(&id_num)?;
                let rank = self.rank_document(&pattern_tokens, tokens)?;
                let id = self
                    .reverse_ids_map
                    .get(&id_num)
                    // this can go wrong only if something (e.g. delete) leaves us in an
                    // inconsistent state
                    .expect("id at id_num should be there")
                    .to_owned();
                Some(RankedResult { id_num, id, rank })
            })
            .collect();

        results.sort_by(|lhs, rhs| self.compare_ranked_results(lhs, rhs));
        results.truncate(self.options.limit);
        results
    }

    fn collect_candidates(&self, pattern_tokens: &[String]) -> HashSet<usize> {
        let mut candidates = HashSet::new();
        for pattern_token in pattern_tokens {
            for (token, id_nums) in &self.reverse_map {
                if self.token_similarity(pattern_token, token).is_some() {
                    for id_num in id_nums {
                        candidates.insert(*id_num);
                    }
                }
            }
        }
        candidates
    }

    fn rank_document(&self, pattern_tokens: &[String], tokens: &[String]) -> Option<Rank> {
        if tokens.is_empty() {
            return None;
        }

        let mut matches = Vec::new();
        for (query_index, pattern_token) in pattern_tokens.iter().enumerate() {
            for (doc_index, token) in tokens.iter().enumerate() {
                if let Some(similarity) = self.token_similarity(pattern_token, token) {
                    matches.push(TokenMatch {
                        query_index,
                        doc_index,
                        score: similarity.score,
                        typo_cost: similarity.typo_cost,
                        exact: similarity.exact,
                    });
                }
            }
        }
        matches.sort_by(Self::compare_token_matches);

        let mut used_queries = vec![false; pattern_tokens.len()];
        let mut used_tokens = vec![false; tokens.len()];
        let mut selected = Vec::new();
        for token_match in matches {
            if !used_queries[token_match.query_index] && !used_tokens[token_match.doc_index] {
                used_queries[token_match.query_index] = true;
                used_tokens[token_match.doc_index] = true;
                selected.push(token_match);
            }
        }

        if selected.is_empty() {
            return None;
        }

        selected.sort_by_key(|token_match| token_match.query_index);
        Some(Rank::from_matches(
            &selected,
            pattern_tokens.len(),
            tokens.len(),
        ))
    }

    fn compare_ranked_results(&self, lhs: &RankedResult<Id>, rhs: &RankedResult<Id>) -> Ordering {
        rhs.rank
            .score
            .partial_cmp(&lhs.rank.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| lhs.id_num.cmp(&rhs.id_num))
    }

    fn compare_token_matches(lhs: &TokenMatch, rhs: &TokenMatch) -> Ordering {
        rhs.exact
            .cmp(&lhs.exact)
            .then_with(|| lhs.typo_cost.cmp(&rhs.typo_cost))
            .then_with(|| rhs.score.partial_cmp(&lhs.score).unwrap_or(Ordering::Equal))
            .then_with(|| lhs.doc_index.cmp(&rhs.doc_index))
            .then_with(|| lhs.query_index.cmp(&rhs.query_index))
    }

    fn token_similarity(&self, pattern_token: &str, token: &str) -> Option<TokenSimilarity> {
        let exact = pattern_token == token;
        let score = jaro_winkler(token, pattern_token);
        let typo_cost = Self::typo_cost_from_score(score, max(token.len(), pattern_token.len()));

        if score > 0.0 {
            Some(TokenSimilarity {
                score,
                typo_cost,
                exact,
            })
        } else {
            None
        }
    }

    fn typo_cost_from_score(score: f64, len: usize) -> usize {
        if score >= 1.0 {
            0
        } else {
            ((1.0 - score) * len as f64).ceil() as usize
        }
    }

    /// Remove an entry by id.
    pub fn remove(&mut self, id: &Id) {
        if let Some(id_num) = self.ids_map.get(id) {
            for token in &self.forward_map[id_num] {
                if let Some(vec) = self.reverse_map.get_mut(token) {
                    vec.retain(|i| i != id_num);
                    if vec.is_empty() {
                        // prune empty token entry to keep the index small
                        self.reverse_map.remove(token);
                    }
                }
            }
            self.forward_map.remove(id_num);
            self.reverse_ids_map.remove(id_num);
            self.ids_map.remove(id);
        };
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.id_num_counter = 0;
        self.ids_map.clear();
        self.reverse_ids_map.clear();
        self.forward_map.clear();
        self.reverse_map.clear();
    }

    fn tokenize(&self, tokens: &[&str]) -> Vec<String> {
        let tokens = self.normalize_tokens(tokens);

        let mut tokens: Vec<String> = if self.options.split_whitespace {
            tokens
                .iter()
                .flat_map(|token| token.split_whitespace())
                .map(|token| token.to_string())
                .collect()
        } else {
            tokens
        };

        for separator in &self.options.separators {
            tokens = tokens
                .iter()
                .flat_map(|token| token.split_terminator(separator.as_str()))
                .map(|token| token.to_string())
                .collect();
        }

        tokens.retain(|token| !token.is_empty());

        tokens
    }

    fn normalize_tokens(&self, tokens: &[&str]) -> Vec<String> {
        let mut tokens: Vec<String> = tokens
            .iter()
            .map(|token| {
                if self.options.case_sensitive {
                    token.to_string()
                } else {
                    token.to_lowercase()
                }
            })
            .collect();

        tokens.retain(|token| !token.is_empty());

        tokens
    }
}

/// Options for configuring the search index.
///
/// # Examples
///
/// ```
/// use simsearch::{Options, Index};
///
/// let mut engine: Index<usize> = Index::with_options(
///     Options::new().case_sensitive(true));
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Options {
    case_sensitive: bool,
    split_whitespace: bool,
    separators: Vec<String>,
    limit: usize,
}

impl Options {
    /// Creates a default configuration.
    pub fn new() -> Self {
        Options {
            case_sensitive: false,
            split_whitespace: true,
            separators: vec![],
            limit: 10,
        }
    }

    /// Sets whether the index is case sensitive.
    ///
    /// Defaults to `false`.
    pub fn case_sensitive(self, case_sensitive: bool) -> Self {
        Options {
            case_sensitive,
            ..self
        }
    }

    /// Sets whether the index splits tokens on whitespace.
    /// Whitespace includes spaces, tabs, returns, and similar characters.
    ///
    /// See also [`std::str::split_whitespace()`](https://doc.rust-lang.org/std/primitive.str.html#method.split_whitespace).
    ///
    /// Defaults to `true`.
    pub fn split_whitespace(self, split_whitespace: bool) -> Self {
        Options {
            split_whitespace,
            ..self
        }
    }

    /// Sets custom token separators.
    ///
    /// This option enables the tokenizer to split indexed fields and search
    /// queries by the extra list of custom separators.
    ///
    /// Defaults to `&[]`.
    ///
    /// # Examples
    /// ```
    /// use simsearch::{Options, Index};
    ///
    /// let mut engine: Index<usize> = Index::with_options(
    ///     Options::new().separators(vec!["/".to_string(), "\\".to_string()]));
    ///
    /// engine.insert(1, "the old/man/and/the sea");
    ///
    /// let results = engine.search("old");
    ///
    /// assert_eq!(results[0].id, 1);
    /// ```
    pub fn separators(self, separators: Vec<String>) -> Self {
        Options { separators, ..self }
    }

    /// Sets the maximum number of results returned by a search.
    ///
    /// Defaults to `10`.
    pub fn limit(self, limit: usize) -> Self {
        Options { limit, ..self }
    }
}

impl<Id> Default for Index<Id>
where
    Id: Eq + Clone + Hash,
{
    fn default() -> Self {
        Index::new()
    }
}

impl Default for Options {
    fn default() -> Self {
        Options::new()
    }
}
