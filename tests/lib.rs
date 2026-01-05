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
