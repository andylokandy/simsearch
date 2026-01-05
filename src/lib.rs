//! A simple and lightweight fuzzy search engine that works in memory, searching for
//! similar strings (a pun here).
//!
//! # Examples
//!
//! ```
//! use simsearch::SimSearch;
//!
//! let mut engine: SimSearch<u32> = SimSearch::new();
//!
//! engine.insert(1, "Things Fall Apart");
//! engine.insert(2, "The Old Man and the Sea");
//! engine.insert(3, "James Joyce");
//!
//! let results: Vec<u32> = engine.search("thngs");
//!
//! assert_eq!(results, &[1]);
//! ```
//!
//! By default, Jaro-Winkler distance is used. An alternative Levenshtein distance, which is
//! SIMD-accelerated but only works for ASCII byte strings, can be specified with `SearchOptions`:
//!
//! ```
//! use simsearch::{SimSearch, SearchOptions};
//!
//! let options = SearchOptions::new().levenshtein(true);
//! let mut engine: SimSearch<u32> = SimSearch::new_with(options);
//! ```

use std::cmp::{max, Ordering};
use std::collections::HashMap;
use std::f64;
use std::hash::Hash;

use strsim::jaro_winkler;
use triple_accel::levenshtein::levenshtein_simd_k;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The simple search engine.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SimSearch<Id>
where
    Id: Eq + PartialEq + Clone + Hash + Ord,
{
    option: SearchOptions,
    id_num_counter: usize,
    ids_map: HashMap<Id, usize>,
    reverse_ids_map: HashMap<usize, Id>,
    forward_map: HashMap<usize, Vec<String>>,
    reverse_map: HashMap<String, Vec<usize>>,
}

impl<Id> SimSearch<Id>
where
    Id: Eq + PartialEq + Clone + Hash + Ord,
{
    /// Creates search engine with default options.
    pub fn new() -> Self {
        Self::new_with(SearchOptions::new())
    }

    /// Creates search engine with custom options.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::{SearchOptions, SimSearch};
    ///
    /// let mut engine: SimSearch<usize> = SimSearch::new_with(
    ///     SearchOptions::new().case_sensitive(true));
    /// ```
    pub fn new_with(option: SearchOptions) -> Self {
        SimSearch {
            option,
            id_num_counter: 0,
            ids_map: HashMap::new(),
            reverse_ids_map: HashMap::new(),
            forward_map: HashMap::new(),
            reverse_map: HashMap::new(),
        }
    }

    /// Inserts an entry into search engine.
    ///
    /// Input will be tokenized according to the search option.
    /// By default whitespaces(including tabs) are considered as stop words,
    /// you can change the behavior by providing `SearchOptions`.
    ///
    /// Insert with an existing id updates the content.
    ///
    /// **Note that** id is not searchable. Add id to the contents if you would
    /// like to perform search on it.
    ///
    /// Additionally, note that content must be an ASCII string if Levenshtein
    /// distance is used.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::{SearchOptions, SimSearch};
    ///
    /// let mut engine: SimSearch<&str> = SimSearch::new_with(
    ///     SearchOptions::new().stop_words(vec![",".to_string(), ".".to_string()]));
    ///
    /// engine.insert("BoJack Horseman", "BoJack Horseman, an American
    /// adult animated comedy-drama series created by Raphael Bob-Waksberg.
    /// The series stars Will Arnett as the title character,
    /// with a supporting cast including Amy Sedaris,
    /// Alison Brie, Paul F. Tompkins, and Aaron Paul.");
    /// ```
    pub fn insert(&mut self, id: Id, content: &str) {
        self.insert_tokens(id, &[content])
    }

    /// Inserts entry tokens into search engine.
    ///
    /// Search engine also applies tokenizer to the
    /// provided tokens. Use this method when you have
    /// special tokenization rules in addition to the built-in ones.
    ///
    /// Insert with an existing id updates the content.
    ///
    /// **Note that** id is not searchable. Add id to the contents if you would
    /// like to perform search on it.
    ///
    /// Additionally, note that each token must be an ASCII string if Levenshtein
    /// distance is used.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::SimSearch;
    ///
    /// let mut engine: SimSearch<&str> = SimSearch::new();
    ///
    /// engine.insert_tokens("Arya Stark", &["Arya Stark", "a fictional
    /// character in American author George R. R", "portrayed by English actress."]);
    pub fn insert_tokens(&mut self, id: Id, tokens: &[&str]) {
        self.remove(&id);

        let id_num = self.id_num_counter;
        self.ids_map.insert(id.clone(), id_num);
        self.reverse_ids_map.insert(id_num, id);
        self.id_num_counter += 1;

        let mut tokens = self.tokenize(tokens);
        tokens.sort();

        for token in tokens.clone() {
            self.reverse_map
                .entry(token)
                .or_insert_with(|| Vec::with_capacity(1))
                .push(id_num);
        }

        self.forward_map.insert(id_num, tokens);
    }

    /// Searches pattern and returns ids sorted by relevance.
    ///
    /// Pattern will be tokenized according to the search option.
    /// By default whitespaces(including tabs) are considered as stop words,
    /// you can change the behavior by providing `SearchOptions`.
    ///
    /// Additionally, note that pattern must be an ASCII string if Levenshtein
    /// distance is used.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::SimSearch;
    ///
    /// let mut engine: SimSearch<u32> = SimSearch::new();
    ///
    /// engine.insert(1, "Things Fall Apart");
    /// engine.insert(2, "The Old Man and the Sea");
    /// engine.insert(3, "James Joyce");
    ///
    /// let results: Vec<u32> = engine.search("thngs apa");
    ///
    /// assert_eq!(results, &[1]);
    pub fn search(&self, pattern: &str) -> Vec<Id> {
        self.search_tokens(&[pattern])
    }

    /// Searches pattern tokens and returns ids sorted by relevance.
    ///
    /// Search engine also applies tokenizer to the
    /// provided tokens. Use this method when you have
    /// special tokenization rules in addition to the built-in ones.
    ///
    /// Additionally, note that each pattern token must be an ASCII
    /// string if Levenshtein distance is used.
    ///
    /// # Examples
    ///
    /// ```
    /// use simsearch::SimSearch;
    ///
    /// let mut engine: SimSearch<u32> = SimSearch::new();
    ///
    /// engine.insert(1, "Things Fall Apart");
    /// engine.insert(2, "The Old Man and the Sea");
    /// engine.insert(3, "James Joyce");
    ///
    /// let results: Vec<u32> = engine.search_tokens(&["thngs", "apa"]);
    ///
    /// assert_eq!(results, &[1]);
    /// ```
    pub fn search_tokens(&self, pattern_tokens: &[&str]) -> Vec<Id> {
        let mut pattern_tokens = self.tokenize(pattern_tokens);
        pattern_tokens.sort();

        let mut token_scores: HashMap<&str, f64> = HashMap::new();

        for pattern_token in pattern_tokens {
            for token in self.reverse_map.keys() {
                let score = if self.option.levenshtein {
                    let len = max(token.len(), pattern_token.len()) as f64;
                    // calculate k (based on the threshold) to bound the Levenshtein distance
                    let k = ((1.0 - self.option.threshold) * len).ceil() as u32;

                    // cheap prefilter: if length difference already exceeds k,
                    // levenshtein_simd_k would return None, so skip calling it.
                    let tlen = token.len() as i32;
                    let plen = pattern_token.len() as i32;
                    if ((tlen - plen).abs() as u32) > k {
                        f64::MIN
                    } else {
                        // levenshtein_simd_k only works on ASCII byte slices, so the token strings
                        // are directly treated as byte slices
                        match levenshtein_simd_k(token.as_bytes(), pattern_token.as_bytes(), k) {
                            Some(dist) => 1.0 - if len == 0.0 { 0.0 } else { (dist as f64) / len },
                            None => f64::MIN,
                        }
                    }
                } else {
                    jaro_winkler(token, &pattern_token)
                };

                if score > self.option.threshold {
                    token_scores.insert(token, score);
                }
            }
        }

        let mut result_scores: HashMap<usize, f64> = HashMap::new();

        for (token, score) in token_scores.drain() {
            for id_num in &self.reverse_map[token] {
                *result_scores.entry(*id_num).or_insert(0.) += score;
            }
        }

        let mut result_scores: Vec<(f64, Id)> = result_scores
            .drain()
            .map(|(id_num, score)| {
                let id = self
                    .reverse_ids_map
                    .get(&id_num)
                    // this can go wrong only if something (e.g. delete) leaves us in an
                    // inconsistent state
                    .expect("id at id_num should be there")
                    .to_owned();
                (score, id)
            })
            .collect();

        result_scores.sort_by(|(lhs_score, lhs_id), (rhs_score, rhs_id)| {
            match rhs_score.partial_cmp(lhs_score).unwrap() {
                Ordering::Equal => lhs_id.cmp(rhs_id),
                ord => ord,
            }
        });

        let result_ids: Vec<Id> = result_scores.into_iter().map(|(_, id)| id).collect();

        result_ids
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
        let tokens: Vec<String> = tokens
            .iter()
            .map(|token| {
                if self.option.case_sensitive {
                    token.to_string()
                } else {
                    token.to_lowercase()
                }
            })
            .collect();

        let mut tokens: Vec<String> = if self.option.stop_whitespace {
            tokens
                .iter()
                .flat_map(|token| token.split_whitespace())
                .map(|token| token.to_string())
                .collect()
        } else {
            tokens
        };

        for stop_word in &self.option.stop_words {
            tokens = tokens
                .iter()
                .flat_map(|token| token.split_terminator(stop_word.as_str()))
                .map(|token| token.to_string())
                .collect();
        }

        tokens.retain(|token| !token.is_empty());

        

        tokens
    }
}

/// Options and flags that configuring the search engine.
///
/// # Examples
///
/// ```
/// use simsearch::{SearchOptions, SimSearch};
///
/// let mut engine: SimSearch<usize> = SimSearch::new_with(
///     SearchOptions::new().case_sensitive(true));
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SearchOptions {
    case_sensitive: bool,
    stop_whitespace: bool,
    stop_words: Vec<String>,
    threshold: f64,
    levenshtein: bool,
}

impl SearchOptions {
    /// Creates a default configuration.
    pub fn new() -> Self {
        SearchOptions {
            case_sensitive: false,
            stop_whitespace: true,
            stop_words: vec![],
            threshold: 0.8,
            levenshtein: false,
        }
    }

    /// Sets whether search engine is case sensitive or not.
    ///
    /// Defaults to `false`.
    pub fn case_sensitive(self, case_sensitive: bool) -> Self {
        SearchOptions {
            case_sensitive,
            ..self
        }
    }

    /// Sets the whether search engine splits tokens on whitespace or not.
    /// The **whitespace** here includes tab, returns and so forth.
    ///
    /// See also [`std::str::split_whitespace()`](https://doc.rust-lang.org/std/primitive.str.html#method.split_whitespace).
    ///
    /// Defaults to `true`.
    pub fn stop_whitespace(self, stop_whitespace: bool) -> Self {
        SearchOptions {
            stop_whitespace,
            ..self
        }
    }

    /// Sets the custom token stop word.
    ///
    /// This option enables tokenizer to split contents
    /// and search words by the extra list of custom stop words.
    ///
    /// Defaults to `&[]`.
    ///
    /// # Examples
    /// ```
    /// use simsearch::{SearchOptions, SimSearch};
    ///
    /// let mut engine: SimSearch<usize> = SimSearch::new_with(
    ///     SearchOptions::new().stop_words(vec!["/".to_string(), "\\".to_string()]));
    ///
    /// engine.insert(1, "the old/man/and/the sea");
    ///
    /// let results = engine.search("old");
    ///
    /// assert_eq!(results, &[1]);
    /// ```
    pub fn stop_words(self, stop_words: Vec<String>) -> Self {
        SearchOptions { stop_words, ..self }
    }

    /// Sets the threshold for search scoring.
    ///
    /// Search results will be sorted by their Jaro winkler similarity scores.
    /// Scores ranges from 0 to 1 where the 1 indicates the most relevant.
    /// Only the entries with scores greater than the threshold will be returned.
    ///
    /// Defaults to `0.8`.
    pub fn threshold(self, threshold: f64) -> Self {
        SearchOptions { threshold, ..self }
    }

    /// Sets whether Levenshtein distance, which is SIMD-accelerated, should be
    /// used instead of the default Jaro-Winkler distance.
    ///
    /// The implementation of Levenshtein distance is very fast but cannot handle Unicode
    /// strings, unlike the default Jaro-Winkler distance. The strings are treated as byte
    /// slices with Levenshtein distance, which means that the calculated score may be
    /// incorrectly lower for Unicode strings, where each character is represented with
    /// multiple bytes.
    ///
    /// Defaults to `false`.
    pub fn levenshtein(self, levenshtein: bool) -> Self {
        SearchOptions {
            levenshtein,
            ..self
        }
    }
}

impl<Id> Default for SimSearch<Id>
where
    Id: Eq + PartialEq + Clone + Hash + Ord,
{
    fn default() -> Self {
        SimSearch::new()
    }
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions::new()
    }
}
