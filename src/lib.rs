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

const QUALITY_WEIGHT: f64 = 0.68;
const COVERAGE_WEIGHT: f64 = 0.14;
const PROXIMITY_WEIGHT: f64 = 0.07;
const EXACTNESS_WEIGHT: f64 = 0.04;
const POSITION_WEIGHT: f64 = 0.03;
const SPECIFICITY_WEIGHT: f64 = 0.04;
const ASSIGNMENT_BEAM_WIDTH: usize = 64;

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
    typo_map: HashMap<String, HashSet<String>>,
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

#[derive(Debug, Clone)]
struct TermCandidate {
    term: String,
    similarity: TokenSimilarity,
}

#[derive(Debug, Clone)]
struct AssignmentState {
    selected: Vec<TokenMatch>,
    used_tokens: Vec<bool>,
    rank: Rank,
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
            terms: BTreeSet::new(),
            typo_map: HashMap::new(),
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

    /// Inserts an entry with multiple searchable parts into the index.
    ///
    /// Each part is tokenized with the same built-in tokenizer used by
    /// [`Index::insert`]. This is useful when an item has several searchable
    /// parts, such as a title, author, tags, or aliases.
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
        let parts: Vec<String> = parts
            .into_iter()
            .map(|part| part.as_ref().to_string())
            .collect();
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let tokens = self.tokenize(&parts);
        self.insert_normalized_tokens(id, tokens)
    }

    fn insert_normalized_tokens(&mut self, id: Id, tokens: Vec<String>) {
        self.remove(&id);

        let id_num = self.id_num_counter;
        self.ids_map.insert(id.clone(), id_num);
        self.reverse_ids_map.insert(id_num, id);
        self.id_num_counter += 1;

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
    /// Pattern will be tokenized according to the search option.
    /// By default whitespaces(including tabs) are considered as separators,
    /// you can change the behavior by providing `Options`.
    /// Search matches exact terms, the last query term as a prefix, and
    /// typo-tolerant terms when those options are enabled.
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
                                typo_cost: candidate.similarity.typo_cost,
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

        let mut selected = Self::select_best_matches(&matches_by_query, tokens.len());
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

    fn select_best_matches(
        matches_by_query: &[Vec<TokenMatch>],
        doc_len: usize,
    ) -> Vec<TokenMatch> {
        let query_len = matches_by_query.len();
        let mut query_order: Vec<usize> = (0..query_len).collect();
        query_order.sort_by(|lhs, rhs| {
            matches_by_query[*lhs]
                .len()
                .cmp(&matches_by_query[*rhs].len())
                .then_with(|| lhs.cmp(rhs))
        });

        let mut states = vec![AssignmentState {
            selected: Vec::new(),
            used_tokens: vec![false; doc_len],
            rank: Rank { score: 0.0 },
        }];

        for query_index in query_order {
            let matches = &matches_by_query[query_index];
            if matches.is_empty() {
                continue;
            }

            let mut next_states = Vec::new();
            for state in &states {
                next_states.push(state.clone());

                for token_match in matches {
                    if state.used_tokens[token_match.doc_index] {
                        continue;
                    }

                    let mut selected = state.selected.clone();
                    selected.push(token_match.clone());
                    selected.sort_by_key(|selected_match| selected_match.query_index);

                    let mut used_tokens = state.used_tokens.clone();
                    used_tokens[token_match.doc_index] = true;
                    let rank = Rank::from_matches(&selected, query_len, doc_len);

                    next_states.push(AssignmentState {
                        selected,
                        used_tokens,
                        rank,
                    });
                }
            }

            next_states.sort_by(Self::compare_assignment_states);
            next_states.truncate(ASSIGNMENT_BEAM_WIDTH);
            states = next_states;
        }

        states.sort_by(Self::compare_assignment_states);
        states
            .into_iter()
            .next()
            .map(|state| state.selected)
            .unwrap_or_default()
    }

    fn compare_assignment_states(lhs: &AssignmentState, rhs: &AssignmentState) -> Ordering {
        rhs.rank
            .score
            .partial_cmp(&lhs.rank.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| rhs.selected.len().cmp(&lhs.selected.len()))
            .then_with(|| {
                let lhs_score = lhs
                    .selected
                    .iter()
                    .map(|token_match| token_match.score)
                    .sum::<f64>();
                let rhs_score = rhs
                    .selected
                    .iter()
                    .map(|token_match| token_match.score)
                    .sum::<f64>();
                rhs_score.partial_cmp(&lhs_score).unwrap_or(Ordering::Equal)
            })
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
            .then_with(|| lhs.typo_cost.cmp(&rhs.typo_cost))
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
                    typo_cost: 0,
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
                        typo_cost: 0,
                        exact: false,
                    },
                );
            }
        }

        if self.options.typo_tolerance {
            for term in self.typo_terms(pattern_token) {
                if term == pattern_token {
                    continue;
                }

                let max_typos = Self::allowed_typos(pattern_token);
                if let Some(typo_cost) = Self::edit_distance_at_most(pattern_token, term, max_typos)
                {
                    Self::insert_candidate(
                        &mut candidates,
                        term,
                        TokenSimilarity {
                            score: Self::typo_score(pattern_token, term, typo_cost),
                            typo_cost,
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
            .then_with(|| lhs.typo_cost.cmp(&rhs.typo_cost))
            .then_with(|| rhs.exact.cmp(&lhs.exact))
    }

    fn prefix_terms(&self, prefix: &str) -> Vec<&str> {
        if prefix.is_empty() {
            return Vec::new();
        }

        self.terms
            .range(prefix.to_string()..)
            .take_while(|term| term.starts_with(prefix))
            .map(String::as_str)
            .collect()
    }

    fn typo_terms(&self, pattern_token: &str) -> Vec<&str> {
        let mut terms = HashSet::new();
        for variant in Self::deletion_variants(pattern_token, Self::allowed_typos(pattern_token)) {
            if let Some(candidates) = self.typo_map.get(&variant) {
                for term in candidates {
                    terms.insert(term.as_str());
                }
            }
        }

        terms.into_iter().collect()
    }

    fn prefix_score(prefix: &str, term: &str) -> f64 {
        let prefix_len = prefix.chars().count();
        let term_len = term.chars().count().max(1);
        0.9 + 0.1 * prefix_len as f64 / term_len as f64
    }

    fn typo_score(pattern_token: &str, term: &str, typo_cost: usize) -> f64 {
        let len = pattern_token
            .chars()
            .count()
            .max(term.chars().count())
            .max(1);
        1.0 - typo_cost as f64 / (len + 1) as f64
    }

    fn allowed_typos(token: &str) -> usize {
        match token.chars().count() {
            0..=4 => 0,
            5..=8 => 1,
            _ => 2,
        }
    }

    fn add_term(&mut self, term: &str) {
        self.terms.insert(term.to_string());
        for variant in Self::deletion_variants(term, Self::allowed_typos(term)) {
            self.typo_map
                .entry(variant)
                .or_default()
                .insert(term.to_string());
        }
    }

    fn remove_term(&mut self, term: &str) {
        self.terms.remove(term);
        for variant in Self::deletion_variants(term, Self::allowed_typos(term)) {
            if let Some(terms) = self.typo_map.get_mut(&variant) {
                terms.remove(term);
                if terms.is_empty() {
                    self.typo_map.remove(&variant);
                }
            }
        }
    }

    fn deletion_variants(token: &str, max_deletions: usize) -> HashSet<String> {
        let mut variants = HashSet::from([token.to_string()]);
        let mut current = HashSet::from([token.to_string()]);

        for _ in 0..max_deletions {
            let mut next = HashSet::new();
            for variant in &current {
                let chars: Vec<char> = variant.chars().collect();
                for index in 0..chars.len() {
                    let mut deleted = String::with_capacity(variant.len());
                    for (char_index, character) in chars.iter().enumerate() {
                        if char_index != index {
                            deleted.push(*character);
                        }
                    }
                    if variants.insert(deleted.clone()) {
                        next.insert(deleted);
                    }
                }
            }
            current = next;
        }

        variants
    }

    fn edit_distance_at_most(lhs: &str, rhs: &str, max_distance: usize) -> Option<usize> {
        let lhs: Vec<char> = lhs.chars().collect();
        let rhs: Vec<char> = rhs.chars().collect();

        if lhs.len().abs_diff(rhs.len()) > max_distance {
            return None;
        }

        let mut distances = vec![vec![0; rhs.len() + 1]; lhs.len() + 1];
        for (lhs_index, row) in distances.iter_mut().enumerate() {
            row[0] = lhs_index;
        }
        for (rhs_index, distance) in distances[0].iter_mut().enumerate() {
            *distance = rhs_index;
        }

        for (lhs_index, lhs_char) in lhs.iter().enumerate() {
            let row = lhs_index + 1;
            let mut row_min = distances[row][0];

            for (rhs_index, rhs_char) in rhs.iter().enumerate() {
                let column = rhs_index + 1;
                let substitution_cost = usize::from(lhs_char != rhs_char);
                distances[row][column] = (distances[row - 1][column] + 1)
                    .min(distances[row][column - 1] + 1)
                    .min(distances[row - 1][column - 1] + substitution_cost);

                if row > 1
                    && column > 1
                    && lhs[row - 1] == rhs[column - 2]
                    && lhs[row - 2] == rhs[column - 1]
                {
                    distances[row][column] =
                        distances[row][column].min(distances[row - 2][column - 2] + 1);
                }

                row_min = row_min.min(distances[row][column]);
            }

            if row_min > max_distance {
                return None;
            }
        }

        let distance = distances[lhs.len()][rhs.len()];
        (distance <= max_distance).then_some(distance)
    }

    /// Remove an entry by id.
    pub fn remove(&mut self, id: &Id) {
        if let Some(id_num) = self.ids_map.get(id).copied() {
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
            self.ids_map.remove(id);
            self.reverse_ids_map.remove(&id_num);
        };
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.id_num_counter = 0;
        self.ids_map.clear();
        self.reverse_ids_map.clear();
        self.forward_map.clear();
        self.reverse_map.clear();
        self.terms.clear();
        self.typo_map.clear();
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
///     Options::new().limit(20));
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Options {
    limit: usize,
    prefix_search: bool,
    typo_tolerance: bool,
    case_sensitive: bool,
    split_whitespace: bool,
    separators: Vec<String>,
}

impl Options {
    /// Creates a default configuration.
    pub fn new() -> Self {
        Options {
            limit: 10,
            prefix_search: true,
            typo_tolerance: true,
            case_sensitive: false,
            split_whitespace: true,
            separators: vec![],
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

    /// Sets whether search tolerates typos using bounded edit distance.
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
    /// This option enables the tokenizer to split indexed parts and search
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
