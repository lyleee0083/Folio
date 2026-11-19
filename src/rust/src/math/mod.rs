mod recognize;
mod render;

use pulldown_cmark::Options;
use std::borrow::Cow;
use unicode_width::UnicodeWidthStr;

pub(super) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub struct Formula {
    pub text: String,
    pub display: bool,
}

pub struct Mask<'a> {
    pub markdown: Cow<'a, str>,
    pub prefix: String,
    pub formulas: Vec<Formula>,
}

impl<'a> Mask<'a> {
    pub fn new(source: &'a str) -> Self {
        let scan = recognize::MathMarkdown::new(
            source,
            Options::ENABLE_TABLES
                | Options::ENABLE_TASKLISTS
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS,
            None,
        );
        let mut mask = Self {
            markdown: Cow::Borrowed(source),
            prefix: "\u{e000}Folio".into(),
            formulas: vec![],
        };
        if scan.replacements.is_empty() {
            return mask;
        }
        while source.contains(&mask.prefix) {
            mask.prefix.push('\u{e000}');
        }
        let mut previous = 0;
        let mut markdown = String::with_capacity(source.len());
        let mut display = scan.display_ranges.iter().peekable();
        for (index, (range, rendered)) in scan.replacements.into_iter().enumerate() {
            while display
                .peek()
                .is_some_and(|candidate| candidate.start < range.start)
            {
                display.next();
            }
            let centered = display.peek().is_some_and(|candidate| **candidate == range);
            markdown.push_str(&source[previous..range.start]);
            markdown.push_str(&mask.prefix);
            markdown.push_str(&index.to_string());
            markdown.push('\u{e001}');
            previous = range.end;
            mask.formulas.push(Formula {
                text: rendered,
                display: centered,
            });
        }
        markdown.push_str(&source[previous..]);
        mask.markdown = Cow::Owned(markdown);
        mask
    }
}
