use std::fs::File;

use simsearch::{SearchOptions, SimSearch};

fn load_books() -> Vec<String> {
    let mut file = File::open("./books.json").unwrap();
    let json: serde_json::Value = serde_json::from_reader(&mut file).unwrap();
    json.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[divan::bench]
fn add_books(bencher: divan::Bencher) {
    let books = load_books();

    bencher.bench(|| {
        let mut engine = SimSearch::new();

        for title in &books {
            engine.insert(title, title);
        }

        engine
    });
}

#[divan::bench]
fn search_jaro_winkler(bencher: divan::Bencher) {
    let books = load_books();
    let mut engine = SimSearch::new();

    for title in &books {
        engine.insert(title, title);
    }

    bencher.bench(|| engine.search("odl sea"));
}

#[divan::bench]
fn search_levenshtein(bencher: divan::Bencher) {
    let books = load_books();
    let options = SearchOptions::new().levenshtein(true);
    let mut engine = SimSearch::new_with(options);

    for title in &books {
        engine.insert(title, title);
    }

    bencher.bench(|| engine.search("odl sea"));
}

fn main() {
    divan::main();
}
