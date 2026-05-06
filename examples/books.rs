use std::fs::File;

use inquire::ui::{Color, RenderConfig, StyleSheet, Styled};
use inquire::{
    CustomUserError, Text,
    autocompletion::{Autocomplete, Replacement},
};

use simsearch::Index;

fn main() {
    inquire::set_global_render_config(get_render_config());

    Text::new("Search for a book:")
        .with_autocomplete(BookSearcher::load())
        .with_help_message("Try typing 'old man'")
        .with_page_size(10)
        .prompt()
        .ok();
}

#[derive(Clone)]
pub struct BookSearcher {
    engine: Index<String>,
}

impl BookSearcher {
    pub fn load() -> Self {
        let mut file = File::open("./books.json").unwrap();
        let json: serde_json::Value = serde_json::from_reader(&mut file).unwrap();
        let books = json
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let mut engine = Index::new();

        for title in books {
            engine.insert(title.clone(), &title);
        }

        BookSearcher { engine }
    }
}

impl Autocomplete for BookSearcher {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        Ok(self
            .engine
            .search(input)
            .into_iter()
            .map(|hit| format!("{:.3}  {}", hit.score, hit.id))
            .collect())
    }

    fn get_completion(
        &mut self,
        _: &str,
        suggestion: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        Ok(suggestion.and_then(|suggestion| {
            suggestion
                .split_once("  ")
                .map(|(_, title)| title.to_string())
        }))
    }
}

fn get_render_config() -> RenderConfig<'static> {
    RenderConfig {
        prompt_prefix: Styled::new(">").with_fg(Color::LightRed),
        highlighted_option_prefix: Styled::new("*").with_fg(Color::LightYellow),
        option: StyleSheet::new().with_fg(Color::DarkBlue),
        help_message: StyleSheet::new().with_fg(Color::LightYellow),
        ..Default::default()
    }
}
