use std::fs::File;

use inquire::ui::{Color, RenderConfig, StyleSheet, Styled};
use inquire::{
    autocompletion::{Autocomplete, Replacement},
    CustomUserError, Text,
};

use simsearch::SimSearch;

fn main() {
    inquire::set_global_render_config(get_render_config());

    Text::new("Search for a book:")
        .with_autocomplete(BookSearcher::load())
        .with_help_message("Try typing 'old man'")
        .with_page_size(15)
        .prompt()
        .ok();
}

#[derive(Clone)]
pub struct BookSearcher {
    engine: SimSearch<String>,
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
        let mut engine = SimSearch::new();

        for title in books {
            engine.insert(title.clone(), &title);
        }

        BookSearcher { engine }
    }
}

impl Autocomplete for BookSearcher {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        Ok(self.engine.search(input))
    }

    fn get_completion(
        &mut self,
        _: &str,
        _: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        Ok(None)
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
