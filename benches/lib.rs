use std::fs::File;

use simsearch::Index;

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
        let mut engine = Index::new();

        for title in &books {
            engine.insert(title, title);
        }

        engine
    });
}

#[divan::bench]
fn search_typo_tolerant(bencher: divan::Bencher) {
    let books = load_books();
    let mut engine = Index::new();

    for title in &books {
        engine.insert(title, title);
    }

    bencher.bench(|| engine.search("odl sea"));
}

fn main() {
    divan::main();
}
