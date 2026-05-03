use std::{fs::File, sync::LazyLock};

use quickcheck_macros::quickcheck;
use simsearch::{SearchOptions, SimSearch};

static ENGINE: LazyLock<SimSearch<String>> = LazyLock::new(populate_engine);

fn populate_engine() -> SimSearch<String> {
    let mut file = File::open("./books.json").unwrap();
    let json: serde_json::Value = serde_json::from_reader(&mut file).unwrap();
    let books = json
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut engine = SimSearch::new_with(SearchOptions::new().stop_whitespace(true));

    for title in books {
        engine.insert(title.clone(), &title);
    }

    engine
}

#[quickcheck]
fn test_quickcheck(tokens: Vec<String>) {
    ENGINE.search(&tokens.join(" "));
}

#[test]
fn remove_prunes_reverse_map_entries() {
    let mut engine: SimSearch<String> = SimSearch::new();
    let id = "id1".to_string();

    engine.insert(id.clone(), "unique-token");
    // ensure present
    let res = engine.search("unique-token");
    assert_eq!(res, vec![id.clone()]);

    engine.remove(&id);

    // after removal the token should no longer be found
    let res2: Vec<String> = engine.search("unique-token");
    assert!(res2.is_empty());
}

#[test]
fn search_with_scores_returns_ids_and_scores() {
    let mut engine: SimSearch<u32> = SimSearch::new();

    engine.insert(1, "Things Fall Apart");

    let results = engine.search_with_scores("thngs");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1);
    assert!(results[0].1 > 0.0);
}

#[test]
fn search_keeps_same_order_as_scored_search() {
    let mut engine: SimSearch<u32> = SimSearch::new_with(SearchOptions::new().threshold(0.0));

    engine.insert(1, "apple");
    engine.insert(2, "apples");
    engine.insert(3, "banana");

    let ids = engine.search("apple");
    let scored_ids = engine
        .search_with_scores("apple")
        .into_iter()
        .map(|(id, _score)| id)
        .collect::<Vec<_>>();

    assert_eq!(ids, scored_ids);
}

#[test]
fn token_score_uses_best_pattern_token_match() {
    let mut engine: SimSearch<u32> = SimSearch::new_with(SearchOptions::new().threshold(0.0));

    engine.insert(1, "foo");

    let bad_then_exact = engine.search_tokens_with_scores(&["bar", "foo"])[0].1;
    let exact_then_bad = engine.search_tokens_with_scores(&["foo", "bar"])[0].1;

    assert_eq!(bad_then_exact, exact_then_bad);
}
