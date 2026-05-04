use std::{fs::File, sync::LazyLock};

use quickcheck_macros::quickcheck;
use simsearch::{Hit, Index, Options};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct StableId(&'static str);

static ENGINE: LazyLock<Index<String>> = LazyLock::new(populate_engine);

fn populate_engine() -> Index<String> {
    let mut file = File::open("./books.json").unwrap();
    let json: serde_json::Value = serde_json::from_reader(&mut file).unwrap();
    let books = json
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut engine = Index::with_options(Options::new().split_whitespace(true));

    for title in books {
        engine.insert(title.clone(), &title);
    }

    engine
}

fn ids<Id>(hits: Vec<Hit<Id>>) -> Vec<Id> {
    hits.into_iter().map(|hit| hit.id).collect()
}

#[quickcheck]
fn test_quickcheck(tokens: Vec<String>) {
    ENGINE.search(&tokens.join(" "));
}

#[test]
fn remove_prunes_reverse_map_entries() {
    let mut engine: Index<String> = Index::new();
    let id = "id1".to_string();

    engine.insert(id.clone(), "unique-token");
    // ensure present
    let res = ids(engine.search("unique-token"));
    assert_eq!(res, vec![id.clone()]);

    engine.remove(&id);

    // after removal the token should no longer be found
    let res2 = engine.search("unique-token");
    assert!(res2.is_empty());
}

#[test]
fn duplicate_tokens_are_indexed_once_per_entry() {
    let mut engine: Index<String> = Index::new();
    let id = "id1".to_string();

    engine.insert(id.clone(), "apple apple");
    engine.remove(&id);

    let results = engine.search("apple");
    assert!(results.is_empty());
}

#[test]
fn search_returns_normalized_scores() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "old man and the sea");
    engine.insert(2, "old sea");

    let results = engine.search("old sea");

    assert_eq!(results[0].id, 2);
    assert!(
        results
            .iter()
            .all(|result| (0.0..=1.0).contains(&result.score))
    );
    assert!(results[0].score >= results[1].score);
}

#[test]
fn search_results_are_sorted_by_score() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "old sea");
    engine.insert(2, "sea old");
    engine.insert(3, "old man and the sea");

    let results = engine.search("old sea");

    assert!(
        results
            .windows(2)
            .all(|pair| pair[0].score >= pair[1].score)
    );
    assert_eq!(
        results.iter().map(|result| result.id).collect::<Vec<_>>(),
        ids(engine.search("old sea"))
    );
}

#[test]
fn ordered_tokens_rank_before_reversed_tokens() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "old sea");
    engine.insert(2, "sea old");

    let results = ids(engine.search("old sea"));

    assert_eq!(results, vec![1, 2]);
}

#[test]
fn near_tokens_rank_before_far_tokens() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "old sea");
    engine.insert(2, "old man and the sea");

    let results = ids(engine.search("old sea"));

    assert_eq!(results, vec![1, 2]);
}

#[test]
fn equal_ranks_keep_insertion_order_without_ord_ids() {
    let mut engine: Index<StableId> = Index::new();
    let first = StableId("first");
    let second = StableId("second");

    engine.insert(first.clone(), "apple");
    engine.insert(second.clone(), "apple");

    let results = ids(engine.search("apple"));

    assert_eq!(results, vec![first, second]);
}

#[test]
fn query_terms_need_distinct_document_tokens() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "new york");
    engine.insert(2, "newyork");

    let results = engine.search("new york");

    assert_eq!(results[0].id, 1);
}

#[test]
fn insert_many_searches_multiple_fields() {
    let mut engine: Index<u32> = Index::new();

    engine.insert_many(1, ["old sea", "hemingway"]);
    engine.insert_many(2, ["old sea", "steinbeck"]);

    let results = engine.search("hemingway");

    assert_eq!(results[0].id, 1);
}

#[test]
fn insert_many_updates_existing_content() {
    let mut engine: Index<u32> = Index::new();

    engine.insert_many(1, ["old sea", "hemingway"]);
    engine.insert_many(2, ["hemingway"]);
    engine.insert_many(1, ["east of eden", "steinbeck"]);

    let results = engine.search("hemingway");

    assert_eq!(results[0].id, 2);
}

#[test]
fn insert_many_uses_builtin_tokenizer() {
    let mut engine: Index<u32> =
        Index::with_options(Options::new().separators(vec!["/".to_string()]));

    engine.insert_many(1, ["old/man"]);

    let split_results = engine.search("old man");
    let token_results = engine.search("old/man");

    assert_eq!(split_results[0].id, 1);
    assert_eq!(token_results[0].id, 1);
}

#[test]
fn limit_caps_results() {
    let mut engine: Index<u32> = Index::with_options(Options::new().limit(1));

    engine.insert(1, "apple");
    engine.insert(2, "apple");

    let results = engine.search("apple");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, 1);
}

#[test]
fn exact_full_match_ranks_before_longer_match() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "apple banana");
    engine.insert(2, "apple");

    let results = engine.search("apple");

    assert_eq!(results[0].id, 2);
    assert!(results[0].score > results[1].score);
}

#[test]
fn exact_full_match_survives_default_limit() {
    let mut engine: Index<u32> = Index::new();

    for id in 0..10 {
        engine.insert(id, "apple banana");
    }
    engine.insert(10, "apple");

    let results = ids(engine.search("apple"));

    assert_eq!(results[0], 10);
    assert!(results.contains(&10));
}

#[test]
fn exact_query_terms_keep_their_matching_document_tokens() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "ac ab");
    engine.insert(2, "ab ad");

    let results = engine.search("ax ac");

    assert_eq!(results[0].id, 1);
    assert!(results[0].score > results[1].score);
}

#[test]
fn strong_fuzzy_matches_are_assigned_before_weaker_matches() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "ae abac");
    engine.insert(2, "abac ae");

    let results = engine.search("aa ab");

    assert_eq!(results[0].id, 1);
    assert!(results[0].score > results[1].score);
}

#[test]
fn token_assignment_uses_higher_similarity_for_fuzzy_matches() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "ac abxxxxxxxxxxxxxxx");
    engine.insert(2, "ac");

    let results = engine.search("ab");

    assert_eq!(results[0].id, 1);
    assert!(results[0].score > results[1].score);
}

#[test]
fn exact_tokens_rank_before_typos() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "apple");
    engine.insert(2, "appel");

    let results = engine.search("apple");

    assert_eq!(results[0].id, 1);
}

#[test]
fn partial_matches_are_allowed_with_lower_scores() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "old man");

    let results = engine.search("old missing");

    assert_eq!(results[0].id, 1);
    assert!(results[0].score > 0.0);
    assert!(results[0].score < 1.0);
}

#[test]
fn exact_tokens_rank_before_typos_with_higher_scores() {
    let mut engine: Index<u32> = Index::new();

    engine.insert(1, "alpha");
    engine.insert(2, "alpah");

    let results = engine.search("alpha");

    assert_eq!(results[0].id, 1);
    assert!(results[0].score > results[1].score);
}
