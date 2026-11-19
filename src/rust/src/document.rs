use comrak::nodes::{AstNode, ListType, NodeValue, TableAlignment};
use comrak::{parse_document, Arena, Options};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
    pub formula: bool,
    pub marker: bool,
}

#[derive(Clone)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Clone)]
pub enum Body {
    Text(Vec<Span>),
    Heading(u8, Vec<Span>),
    Code {
        language: String,
        text: String,
    },
    Table {
        rows: Vec<Vec<Vec<Span>>>,
        align: Vec<TableAlignment>,
        header: bool,
    },
    Image {
        target: String,
        alt: String,
    },
    Formula(String),
    Rule,
}

#[derive(Clone)]
pub struct Block {
    pub body: Body,
    pub indent: usize,
    pub quote: usize,
    pub marker: String,
}

#[derive(Clone)]
pub struct Heading {
    pub block: usize,
    pub level: u8,
    pub title: String,
    pub id: String,
}

#[derive(Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    pub headings: Vec<Heading>,
    pub word_count: usize,
}

#[derive(Clone, Default)]
struct Context {
    indent: usize,
    quote: usize,
    pending_marker: String,
}

fn text(value: impl Into<String>, style: &Style, result: &mut Vec<Span>) {
    result.push(Span {
        text: value.into(),
        style: style.clone(),
    });
}

// Render parser events to typed document runs. The input file is always actual
// document source, including fenced Markdown examples.
pub fn parse(markdown: &str) -> Document {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.tasklist = true;
    options.extension.autolink = true;
    options.extension.strikethrough = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());

    let math = crate::math::Mask::new(markdown);
    let arena = Arena::new();
    let root = parse_document(&arena, &math.markdown, &options);
    let mut result = Document::default();
    flow(root, &mut Context::default(), &mut result);
    if !math.formulas.is_empty() {
        show_formulas(&mut result, &math);
    }

    let mut unique = HashMap::new();
    for (block_idx, block) in result.blocks.iter().enumerate() {
        if let Body::Heading(level, title) = &block.body {
            let title = plain(title);
            let base = slug(&title);
            let mut id = base.clone();
            let mut suffix = 1;
            while unique.contains_key(&id) {
                id = format!("{base}-{suffix}");
                suffix += 1;
            }
            unique.insert(id.clone(), ());
            result.headings.push(Heading {
                block: block_idx,
                level: *level,
                title,
                id,
            });
        }
        result.word_count += match &block.body {
            Body::Text(spans) | Body::Heading(_, spans) => count(&plain(spans)),
            Body::Code { text, .. } => count(text),
            Body::Table { rows, .. } => rows.iter().flatten().map(|cell| count(&plain(cell))).sum(),
            Body::Image { alt, .. } => count(alt),
            Body::Formula(body) => count(body),
            Body::Rule => 0,
        };
    }
    result
}

fn flow<'a>(node: &'a AstNode<'a>, context: &mut Context, result: &mut Document) {
    match &node.data.borrow().value {
        NodeValue::List(list) => {
            for (index, child) in node.children().enumerate() {
                let marker = match &child.data.borrow().value {
                    NodeValue::TaskItem(task) if task.symbol.is_some() => "☑\u{fe0e}".to_string(),
                    NodeValue::TaskItem(_) => "☐".to_string(),
                    _ if list.list_type == ListType::Ordered => format!("{}.", list.start + index),
                    _ => "•".to_string(),
                };
                let mut item = Context {
                    indent: context.indent + 1,
                    quote: context.quote,
                    pending_marker: marker,
                };
                for element in child.children() {
                    flow(element, &mut item, result);
                }
            }
        }
        NodeValue::BlockQuote => {
            let mut quoted = context.clone();
            quoted.quote += 1;
            for child in node.children() {
                flow(child, &mut quoted, result);
            }
        }
        NodeValue::Paragraph => {
            let first = node.first_child();
            if let Some(image) = first.filter(|child| child.next_sibling().is_none()) {
                if let NodeValue::Image(link) = &image.data.borrow().value {
                    let alt = plain(&runs(image));
                    push(
                        result,
                        context,
                        Body::Image {
                            target: link.url.clone(),
                            alt,
                        },
                    );
                    return;
                }
            }
            push(result, context, Body::Text(runs(node)));
        }
        NodeValue::Heading(heading) => {
            push(result, context, Body::Heading(heading.level, runs(node)))
        }
        NodeValue::CodeBlock(code) => push(
            result,
            context,
            Body::Code {
                language: code
                    .info
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                text: code
                    .literal
                    .strip_suffix('\n')
                    .unwrap_or(&code.literal)
                    .to_owned(),
            },
        ),
        NodeValue::Table(table) => {
            let rows = node
                .children()
                .map(|row| row.children().map(runs).collect())
                .collect();
            push(
                result,
                context,
                Body::Table {
                    rows,
                    align: table.alignments.clone(),
                    header: true,
                },
            );
        }
        NodeValue::ThematicBreak => push(result, context, Body::Rule),
        NodeValue::FrontMatter(raw) => {
            let normalized = raw.trim_matches(['\r', '\n']);
            let yaml = normalized
                .strip_prefix("---")
                .and_then(|value| value.strip_suffix("---"));
            let data = yaml
                .filter(|value| value.len() <= 8192)
                .and_then(|value| yaml_rust2::YamlLoader::load_from_str(value).ok());
            let map = data
                .as_ref()
                .and_then(|docs| docs.first())
                .and_then(yaml_rust2::Yaml::as_hash);
            if let Some(map) = map {
                let rows = map
                    .iter()
                    .filter_map(|(key, value)| {
                        let key = key.as_str()?;
                        let value = yaml_display(value, 0)?;
                        Some(vec![
                            vec![Span {
                                text: key.into(),
                                style: Style {
                                    bold: true,
                                    ..Style::default()
                                },
                            }],
                            vec![Span {
                                text: value,
                                style: Style::default(),
                            }],
                        ])
                    })
                    .collect::<Vec<_>>();
                if rows.len() == map.len() {
                    push(
                        result,
                        context,
                        Body::Table {
                            rows,
                            align: vec![TableAlignment::Left; 2],
                            header: false,
                        },
                    );
                    return;
                }
            }
            push(
                result,
                context,
                Body::Text(vec![Span {
                    text: raw.clone(),
                    style: Style::default(),
                }]),
            );
        }
        NodeValue::HtmlBlock(html) => push(
            result,
            context,
            Body::Text(vec![Span {
                text: html.literal.trim_end_matches('\n').to_owned(),
                style: Style::default(),
            }]),
        ),
        _ => {
            for child in node.children() {
                flow(child, context, result);
            }
        }
    }
}

fn push(result: &mut Document, context: &mut Context, body: Body) {
    result.blocks.push(Block {
        body,
        indent: context.indent,
        quote: context.quote,
        marker: std::mem::take(&mut context.pending_marker),
    });
}

fn runs<'a>(node: &'a AstNode<'a>) -> Vec<Span> {
    let mut result = Vec::new();
    for child in node.children() {
        inline(child, Style::default(), &mut result);
    }
    result
}

fn inline<'a>(node: &'a AstNode<'a>, mut style: Style, out: &mut Vec<Span>) {
    match &node.data.borrow().value {
        NodeValue::Text(value) => return text(value.to_string(), &style, out),
        NodeValue::HtmlInline(value) | NodeValue::Raw(value) => {
            return text(value.clone(), &style, out)
        }
        NodeValue::Code(code) => {
            style.code = true;
            return text(code.literal.clone(), &style, out);
        }
        NodeValue::SoftBreak => return text(" ", &style, out),
        NodeValue::LineBreak => return text("\n", &style, out),
        NodeValue::Strong => style.bold = true,
        NodeValue::Emph => style.italic = true,
        NodeValue::Strikethrough => style.strike = true,
        NodeValue::Link(link) => style.link = Some(link.url.clone()),
        NodeValue::Image(image) => style.link = Some(image.url.clone()),
        _ => {}
    }
    for next in node.children() {
        inline(next, style.clone(), out);
    }
}

pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

fn show_formulas(document: &mut Document, math: &crate::math::Mask<'_>) {
    for block in &mut document.blocks {
        match &mut block.body {
            Body::Text(spans) => {
                let display = if spans.len() == 1 {
                    spans[0]
                        .text
                        .strip_prefix(&math.prefix)
                        .and_then(|part| part.strip_suffix('\u{e001}'))
                        .and_then(|index| index.parse::<usize>().ok())
                        .and_then(|index| math.formulas.get(index))
                        .filter(|formula| formula.display)
                } else {
                    None
                };
                if let Some(formula) = display {
                    block.body = Body::Formula(formula.text.clone());
                } else {
                    unmask(spans, math);
                }
            }
            Body::Heading(_, spans) => unmask(spans, math),
            Body::Table { rows, .. } => rows
                .iter_mut()
                .flatten()
                .for_each(|spans| unmask(spans, math)),
            _ => {}
        }
    }
}

fn unmask(spans: &mut Vec<Span>, math: &crate::math::Mask<'_>) {
    if !spans.iter().any(|span| span.text.contains(&math.prefix)) {
        return;
    }
    for span in std::mem::take(spans) {
        let mut rest = span.text.as_str();
        while let Some((before, marker)) = rest.split_once(&math.prefix) {
            let Some((index, after)) = marker.split_once('\u{e001}') else {
                break;
            };
            let Some(formula) = index
                .parse::<usize>()
                .ok()
                .and_then(|index| math.formulas.get(index))
            else {
                break;
            };
            if !before.is_empty() {
                text(before, &span.style, spans);
            }
            text(
                formula.text.clone(),
                &Style {
                    formula: true,
                    ..span.style.clone()
                },
                spans,
            );
            rest = after;
        }
        if !rest.is_empty() {
            text(rest, &span.style, spans);
        }
    }
}

fn slug(title: &str) -> String {
    title
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            ' ' | '\t' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

fn yaml_display(value: &yaml_rust2::Yaml, depth: usize) -> Option<String> {
    use yaml_rust2::Yaml::*;
    if depth > 3 {
        return None;
    }
    match value {
        String(value) | Real(value) => Some(value.clone()),
        Integer(value) => Some(value.to_string()),
        Boolean(value) => Some(value.to_string()),
        Null => Some("null".into()),
        Array(array) if array.len() <= 64 => Some(
            array
                .iter()
                .map(|item| yaml_display(item, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .join(" · "),
        ),
        _ => None,
    }
}

fn count(text: &str) -> usize {
    let mut active = false;
    let mut count = 0;
    for c in text.chars() {
        if matches!(c, '\u{3400}'..='\u{9fff}' | '\u{20000}'..='\u{323af}') {
            count += 1;
            active = false;
        } else if c.is_alphanumeric() {
            if !active {
                count += 1;
            }
            active = true;
        } else {
            active = false;
        }
    }
    count
}
