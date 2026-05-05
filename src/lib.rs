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

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;
use std::ops::Bound;

use strsim::jaro_winkler;

const QUALITY_WEIGHT: f64 = 0.68;
const COVERAGE_WEIGHT: f64 = 0.14;
const PROXIMITY_WEIGHT: f64 = 0.07;
const EXACTNESS_WEIGHT: f64 = 0.04;
const POSITION_WEIGHT: f64 = 0.03;
const SPECIFICITY_WEIGHT: f64 = 0.04;

type PostingMap = HashMap<usize, Vec<usize>>;

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
    reverse_map: HashMap<String, PostingMap>,
    terms: BTreeSet<String>,
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
    exact: bool,
}

#[derive(Debug, Clone, Copy)]
struct TokenSimilarity {
    score: f64,
    exact: bool,
}

#[derive(Debug, Clone)]
struct TermCandidate {
    term: String,
    similarity: TokenSimilarity,
}

#[derive(Debug, Clone)]
struct AssignmentState {
    selected: Vec<TokenMatch>,
    metrics: AssignmentMetrics,
}

#[derive(Debug, Clone, Copy)]
struct AssignmentMetrics {
    matched_terms: usize,
    score_sum: f64,
    exact_terms: usize,
    proximity_cost: usize,
    first_position: usize,
    last_position: Option<usize>,
}

impl AssignmentState {
    fn new() -> Self {
        AssignmentState {
            selected: Vec::new(),
            metrics: AssignmentMetrics {
                matched_terms: 0,
                score_sum: 0.0,
                exact_terms: 0,
                proximity_cost: 0,
                first_position: usize::MAX,
                last_position: None,
            },
        }
    }

    fn with_match(&self, token_match: TokenMatch) -> Self {
        let mut selected = self.selected.clone();
        selected.push(token_match.clone());

        let proximity_cost = self.metrics.proximity_cost
            + self
                .metrics
                .last_position
                .map(|last_position| token_match.doc_index - last_position - 1)
                .unwrap_or(0);

        AssignmentState {
            selected,
            metrics: AssignmentMetrics {
                matched_terms: self.metrics.matched_terms + 1,
                score_sum: self.metrics.score_sum + token_match.score,
                exact_terms: self.metrics.exact_terms + usize::from(token_match.exact),
                proximity_cost,
                first_position: self.metrics.first_position.min(token_match.doc_index),
                last_position: Some(token_match.doc_index),
            },
        }
    }
}

impl Rank {
    fn from_matches(matches: &[TokenMatch], query_len: usize, doc_len: usize) -> Self {
        let matched_terms = matches.len();
        let proximity_cost = Self::proximity_cost(matches);
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

    fn proximity_cost(matches: &[TokenMatch]) -> usize {
        if matches.len() < 2 {
            return 0;
        }

        let mut cost = 0;
        for window in matches.windows(2) {
            let lhs = window[0].doc_index;
            let rhs = window[1].doc_index;
            cost += rhs - lhs - 1;
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
            terms: BTreeSet::new(),
        }
    }

    /// Inserts an entry into the index.
    ///
    /// The content is tokenized according to the index options.
    /// By default, [`char::is_whitespace`] characters are treated as
    /// separators.
    /// Use [`Options`] to add more separators.
    ///
    /// Inserting an existing ID updates its content.
    ///
    /// The ID itself is not searchable. Include it in the content if you want
    /// searches to match it.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::{Options, Index};
    ///
    /// let mut engine: Index<&str> = Index::with_options(
    ///     Options::new().separators([',', '.', '/']));
    ///
    /// engine.insert("BoJack Horseman", "BoJack Horseman, an American
    /// adult animated comedy-drama series created by Raphael Bob-Waksberg.
    /// The series stars Will Arnett as the title character,
    /// with a supporting cast including Amy Sedaris,
    /// Alison Brie, Paul F. Tompkins, and Aaron Paul.");
    /// ```
    pub fn insert(&mut self, id: Id, content: &str) {
        let tokens = self.tokenize([content]);
        self.insert_normalized_tokens(id, tokens)
    }

    /// Inserts an entry with multiple searchable parts into the index.
    ///
    /// Each part is tokenized with the same built-in tokenizer used by
    /// [`Index::insert`]. This is useful when an item has several searchable
    /// parts, such as a title, author, tags, or aliases.
    ///
    /// Inserting an existing ID updates its content.
    ///
    /// The ID itself is not searchable. Include it in the content if you want
    /// searches to match it.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::Index;
    ///
    /// let mut engine: Index<&str> = Index::new();
    ///
    /// engine.insert_parts("A Game of Thrones", [
    ///     "A Game of Thrones",
    ///     "George R. R. Martin",
    ///     "fantasy",
    /// ]);
    ///
    /// let results = engine.search("martin");
    ///
    /// assert_eq!(results[0].id, "A Game of Thrones");
    /// ```
    pub fn insert_parts<I, S>(&mut self, id: Id, parts: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let tokens = self.tokenize(parts);
        self.insert_normalized_tokens(id, tokens)
    }

    fn insert_normalized_tokens(&mut self, id: Id, tokens: Vec<String>) {
        let id_num = if let Some(id_num) = self.ids_map.get(&id).copied() {
            self.remove_indexed_tokens(id_num);
            id_num
        } else {
            let id_num = self.id_num_counter;
            self.ids_map.insert(id.clone(), id_num);
            self.reverse_ids_map.insert(id_num, id);
            self.id_num_counter += 1;
            id_num
        };

        for (position, token) in tokens.iter().enumerate() {
            if !self.reverse_map.contains_key(token) {
                self.add_term(token);
            }

            self.reverse_map
                .entry(token.clone())
                .or_default()
                .entry(id_num)
                .or_default()
                .push(position);
        }

        self.forward_map.insert(id_num, tokens);
    }

    /// Searches pattern and returns up to [`Options::limit`] hits sorted by
    /// relevance.
    ///
    /// The query is tokenized according to the index options.
    /// By default, [`char::is_whitespace`] characters are treated as
    /// separators.
    /// Use [`Options`] to add more separators.
    /// Search matches exact terms, typo-tolerant terms, and the last query
    /// term as a prefix, including typo-tolerant prefixes when both options are
    /// enabled.
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
        self.search_ranked(self.tokenize([pattern]))
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

        let matches_by_document = self.collect_matches(&pattern_tokens);
        let mut results: Vec<RankedResult<Id>> = matches_by_document
            .into_iter()
            .filter_map(|(id_num, matches)| {
                let tokens = self.forward_map.get(&id_num)?;
                let rank = self.rank_document(&pattern_tokens, tokens, matches)?;
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

    fn collect_matches(&self, pattern_tokens: &[String]) -> HashMap<usize, Vec<TokenMatch>> {
        let mut matches_by_document: HashMap<usize, Vec<TokenMatch>> = HashMap::new();

        for (query_index, pattern_token) in pattern_tokens.iter().enumerate() {
            let is_last_query_token = query_index + 1 == pattern_tokens.len();
            for candidate in self.expand_query_term(pattern_token, is_last_query_token) {
                if let Some(postings) = self.reverse_map.get(&candidate.term) {
                    for (id_num, positions) in postings {
                        let matches = matches_by_document.entry(*id_num).or_default();
                        for doc_index in positions {
                            matches.push(TokenMatch {
                                query_index,
                                doc_index: *doc_index,
                                score: candidate.similarity.score,
                                exact: candidate.similarity.exact,
                            });
                        }
                    }
                }
            }
        }

        matches_by_document
    }

    fn rank_document(
        &self,
        pattern_tokens: &[String],
        tokens: &[String],
        matches: Vec<TokenMatch>,
    ) -> Option<Rank> {
        if tokens.is_empty() {
            return None;
        }

        let mut matches_by_query = vec![Vec::new(); pattern_tokens.len()];

        for token_match in matches {
            matches_by_query[token_match.query_index].push(token_match);
        }

        for matches in &mut matches_by_query {
            matches.sort_by(Self::compare_token_matches);
        }

        let mut selected = Self::select_best_matches(&matches_by_query);
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

    fn select_best_matches(matches_by_query: &[Vec<TokenMatch>]) -> Vec<TokenMatch> {
        let mut states = vec![AssignmentState::new()];

        for matches in matches_by_query {
            if matches.is_empty() {
                continue;
            }

            let mut next_states = states.clone();
            for state in &states {
                for token_match in matches {
                    if state
                        .metrics
                        .last_position
                        .is_some_and(|last_position| token_match.doc_index <= last_position)
                    {
                        continue;
                    }

                    next_states.push(state.with_match(token_match.clone()));
                }
            }

            states = Self::prune_assignment_states(next_states);
        }

        states.sort_by(Self::compare_assignment_states);
        states
            .into_iter()
            .next()
            .map(|state| state.selected)
            .unwrap_or_default()
    }

    fn prune_assignment_states(states: Vec<AssignmentState>) -> Vec<AssignmentState> {
        let mut best_by_last_position: HashMap<Option<usize>, AssignmentState> = HashMap::new();

        for state in states {
            let last_position = state.metrics.last_position;
            if let Some(current) = best_by_last_position.get_mut(&last_position) {
                if Self::compare_assignment_states(&state, current).is_lt() {
                    *current = state;
                }
            } else {
                best_by_last_position.insert(last_position, state);
            }
        }

        let mut states: Vec<AssignmentState> = best_by_last_position.into_values().collect();
        states.sort_by(Self::compare_assignment_states);
        states
    }

    fn compare_assignment_states(lhs: &AssignmentState, rhs: &AssignmentState) -> Ordering {
        rhs.metrics
            .matched_terms
            .cmp(&lhs.metrics.matched_terms)
            .then_with(|| {
                rhs.metrics
                    .score_sum
                    .partial_cmp(&lhs.metrics.score_sum)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| rhs.metrics.exact_terms.cmp(&lhs.metrics.exact_terms))
            .then_with(|| lhs.metrics.proximity_cost.cmp(&rhs.metrics.proximity_cost))
            .then_with(|| lhs.metrics.first_position.cmp(&rhs.metrics.first_position))
            .then_with(|| lhs.metrics.last_position.cmp(&rhs.metrics.last_position))
            .then_with(|| Self::compare_selected_matches(&lhs.selected, &rhs.selected))
    }

    fn compare_selected_matches(lhs: &[TokenMatch], rhs: &[TokenMatch]) -> Ordering {
        for (lhs_match, rhs_match) in lhs.iter().zip(rhs) {
            let ordering = lhs_match
                .query_index
                .cmp(&rhs_match.query_index)
                .then_with(|| lhs_match.doc_index.cmp(&rhs_match.doc_index))
                .then_with(|| {
                    rhs_match
                        .score
                        .partial_cmp(&lhs_match.score)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| rhs_match.exact.cmp(&lhs_match.exact));

            if !ordering.is_eq() {
                return ordering;
            }
        }

        lhs.len().cmp(&rhs.len())
    }

    fn compare_ranked_results(&self, lhs: &RankedResult<Id>, rhs: &RankedResult<Id>) -> Ordering {
        rhs.rank
            .score
            .partial_cmp(&lhs.rank.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| lhs.id_num.cmp(&rhs.id_num))
    }

    fn compare_token_matches(lhs: &TokenMatch, rhs: &TokenMatch) -> Ordering {
        rhs.score
            .partial_cmp(&lhs.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| rhs.exact.cmp(&lhs.exact))
            .then_with(|| lhs.doc_index.cmp(&rhs.doc_index))
            .then_with(|| lhs.query_index.cmp(&rhs.query_index))
    }

    fn expand_query_term(&self, pattern_token: &str, prefix_search: bool) -> Vec<TermCandidate> {
        let mut candidates: HashMap<String, TokenSimilarity> = HashMap::new();

        if self.reverse_map.contains_key(pattern_token) {
            Self::insert_candidate(
                &mut candidates,
                pattern_token,
                TokenSimilarity {
                    score: 1.0,
                    exact: true,
                },
            );
        }

        if self.options.prefix_search && prefix_search {
            for term in self.prefix_terms(pattern_token) {
                if term == pattern_token {
                    continue;
                }

                Self::insert_candidate(
                    &mut candidates,
                    term,
                    TokenSimilarity {
                        score: Self::prefix_score(pattern_token, term),
                        exact: false,
                    },
                );
            }
        }

        if self.options.typo_tolerance {
            for term in &self.terms {
                let term = term.as_str();
                if term == pattern_token {
                    continue;
                }

                let score = Self::typo_score(
                    pattern_token,
                    term,
                    self.options.prefix_search && prefix_search,
                );
                if score >= Self::min_typo_score(pattern_token) {
                    Self::insert_candidate(
                        &mut candidates,
                        term,
                        TokenSimilarity {
                            score,
                            exact: false,
                        },
                    );
                }
            }
        }

        candidates
            .into_iter()
            .map(|(term, similarity)| TermCandidate { term, similarity })
            .collect()
    }

    fn insert_candidate(
        candidates: &mut HashMap<String, TokenSimilarity>,
        term: &str,
        similarity: TokenSimilarity,
    ) {
        candidates
            .entry(term.to_string())
            .and_modify(|current| {
                if Self::compare_similarity(similarity, *current).is_lt() {
                    *current = similarity;
                }
            })
            .or_insert(similarity);
    }

    fn compare_similarity(lhs: TokenSimilarity, rhs: TokenSimilarity) -> Ordering {
        rhs.score
            .partial_cmp(&lhs.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| rhs.exact.cmp(&lhs.exact))
    }

    fn prefix_terms(&self, prefix: &str) -> Vec<&str> {
        if prefix.is_empty() {
            return Vec::new();
        }

        self.terms
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(|term| term.starts_with(prefix))
            .map(String::as_str)
            .collect()
    }

    fn prefix_score(prefix: &str, term: &str) -> f64 {
        let prefix_len = prefix.chars().count();
        let term_len = term.chars().count().max(1);
        0.9 + 0.1 * prefix_len as f64 / term_len as f64
    }

    fn typo_score(pattern_token: &str, term: &str, prefix_search: bool) -> f64 {
        let mut score = jaro_winkler(pattern_token, term);

        if prefix_search {
            score = score.max(Self::prefix_typo_score(pattern_token, term));
        }

        score
    }

    fn prefix_typo_score(pattern_token: &str, term: &str) -> f64 {
        let pattern_len = pattern_token.chars().count();
        if pattern_len == 0 {
            return 0.0;
        }

        let term_len = term.chars().count();
        let min_len = pattern_len.saturating_sub(1).max(1);
        let max_len = (pattern_len + 2).min(term_len);
        let mut score: f64 = 0.0;

        for len in min_len..=max_len {
            let prefix: String = term.chars().take(len).collect();
            score = score.max(jaro_winkler(pattern_token, &prefix));
        }

        score.min(Self::prefix_score(pattern_token, term))
    }

    fn min_typo_score(pattern_token: &str) -> f64 {
        match pattern_token.chars().count() {
            0..=3 => 0.6,
            4..=5 => 0.7,
            _ => 0.75,
        }
    }

    fn add_term(&mut self, term: &str) {
        self.terms.insert(term.to_string());
    }

    fn remove_term(&mut self, term: &str) {
        self.terms.remove(term);
    }

    fn remove_indexed_tokens(&mut self, id_num: usize) {
        if let Some(tokens) = self.forward_map.remove(&id_num) {
            let unique_tokens = tokens.into_iter().collect::<HashSet<_>>();
            for token in unique_tokens {
                if let Some(postings) = self.reverse_map.get_mut(&token) {
                    postings.remove(&id_num);
                    if postings.is_empty() {
                        self.reverse_map.remove(&token);
                        self.remove_term(&token);
                    }
                }
            }
        }
    }

    /// Removes an entry by ID.
    pub fn remove(&mut self, id: &Id) {
        if let Some(id_num) = self.ids_map.remove(id) {
            self.remove_indexed_tokens(id_num);
            self.reverse_ids_map.remove(&id_num);
        }
    }

    /// Clears all entries.
    pub fn clear(&mut self) {
        self.id_num_counter = 0;
        self.ids_map.clear();
        self.reverse_ids_map.clear();
        self.forward_map.clear();
        self.reverse_map.clear();
        self.terms.clear();
    }

    fn tokenize<I, S>(&self, parts: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut tokens = Vec::new();
        for part in parts {
            let part = if self.options.case_sensitive {
                part.as_ref().to_string()
            } else {
                part.as_ref().to_lowercase()
            };

            for token in part
                .split(|ch: char| ch.is_whitespace() || self.options.extra_separators.contains(&ch))
            {
                if !token.is_empty() {
                    tokens.push(token.to_string());
                }
            }
        }

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
///     Options::new().limit(20));
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Options {
    limit: usize,
    prefix_search: bool,
    typo_tolerance: bool,
    case_sensitive: bool,
    extra_separators: Vec<char>,
}

impl Options {
    /// Creates a default configuration.
    pub fn new() -> Self {
        Options {
            limit: 10,
            prefix_search: true,
            typo_tolerance: true,
            case_sensitive: false,
            extra_separators: Vec::new(),
        }
    }

    /// Sets the maximum number of results returned by a search.
    ///
    /// Defaults to `10`.
    pub fn limit(self, limit: usize) -> Self {
        Options { limit, ..self }
    }

    /// Sets whether the last query token can match indexed token prefixes.
    ///
    /// Defaults to `true`.
    pub fn prefix_search(self, prefix_search: bool) -> Self {
        Options {
            prefix_search,
            ..self
        }
    }

    /// Sets whether search tolerates typos using Jaro-Winkler similarity.
    ///
    /// Defaults to `true`.
    pub fn typo_tolerance(self, typo_tolerance: bool) -> Self {
        Options {
            typo_tolerance,
            ..self
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

    /// Adds token separators.
    ///
    /// Indexed parts and search queries are always split by
    /// [`char::is_whitespace`] characters. This option adds more
    /// separators on top of that default behavior.
    ///
    /// Defaults to no extra separators.
    ///
    /// # Examples
    /// ```
    /// use simsearch::{Options, Index};
    ///
    /// let mut engine: Index<usize> = Index::with_options(
    ///     Options::new().separators(['/', '\\']));
    ///
    /// engine.insert(1, "the old/man/and/the sea");
    ///
    /// let results = engine.search("old");
    ///
    /// assert_eq!(results[0].id, 1);
    /// ```
    pub fn separators<I>(self, separators: I) -> Self
    where
        I: IntoIterator<Item = char>,
    {
        Options {
            extra_separators: separators.into_iter().collect(),
            ..self
        }
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
