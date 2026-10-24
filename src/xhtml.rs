//! XHTML (an EPUB content document) → flow blocks.
//!
//! Block structure maps one-to-one where the model has a counterpart
//! (headings, paragraphs, quotes, lists, preformatted code, tables, images,
//! rules); everything else flattens to paragraphs. Inline formatting keeps
//! bold / italic / underline / strike / code and external links. Whitespace
//! collapses the way HTML does, except inside `<pre>`.

use mireader_reader_sdk::{Block, Span, SpanStyle};

/// Resolves an `<img src>` (relative to the document) to a resource key.
pub type ResolveImage<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The XML-predefined entities stay; every HTML named entity a book is
/// likely to use becomes a numeric reference (XHTML parsed as XML has no
/// DTD to define them).
fn normalize_entities(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('&') {
        return text.into();
    }
    const NAMED: &[(&str, u32)] = &[
        ("nbsp", 0xA0), ("ensp", 0x2002), ("emsp", 0x2003), ("thinsp", 0x2009),
        ("mdash", 0x2014), ("ndash", 0x2013), ("hellip", 0x2026), ("lsquo", 0x2018),
        ("rsquo", 0x2019), ("ldquo", 0x201C), ("rdquo", 0x201D), ("laquo", 0xAB),
        ("raquo", 0xBB), ("copy", 0xA9), ("reg", 0xAE), ("trade", 0x2122),
        ("middot", 0xB7), ("bull", 0x2022), ("times", 0xD7), ("divide", 0xF7),
        ("deg", 0xB0), ("para", 0xB6), ("sect", 0xA7), ("shy", 0xAD),
        ("eacute", 0xE9), ("egrave", 0xE8), ("agrave", 0xE0), ("aacute", 0xE1),
        ("ccedil", 0xE7), ("ouml", 0xF6), ("uuml", 0xFC), ("auml", 0xE4),
        ("szlig", 0xDF), ("oacute", 0xF3), ("iacute", 0xED), ("uacute", 0xFA),
        ("ntilde", 0xF1), ("euro", 0x20AC), ("pound", 0xA3), ("yen", 0xA5),
        ("cent", 0xA2), ("frac12", 0xBD), ("frac14", 0xBC), ("frac34", 0xBE),
        ("dagger", 0x2020), ("Dagger", 0x2021), ("prime", 0x2032), ("zwj", 0x200D),
        ("zwnj", 0x200C), ("lrm", 0x200E), ("rlm", 0x200F),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let name_len = tail
            .find(';')
            .filter(|&end| end <= 10 && tail[..end].chars().all(|c| c.is_ascii_alphanumeric()));
        match name_len.and_then(|end| NAMED.iter().find(|(n, _)| *n == &tail[..end]).map(|(_, c)| (end, c))) {
            Some((end, code)) => {
                out.push_str(&format!("&#{code};"));
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out.into()
}

/// MIREADER (round 15, #46): how an in-book link leaves the converter, before
/// the book resolves it.
pub const INBOOK: &str = "inbook:";

/// MIREADER (round 15, #46): the heading each `id` in a section leads to —
/// the element's own text when it is a heading, else its first heading
/// inside, else the next heading after it — for in-book links that land
/// on a fragment. `(id, heading text)`.
pub fn anchor_headings(text: &str) -> Vec<(String, String)> {
    let text = normalize_entities(text);
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let Ok(doc) = roxmltree::Document::parse_with_options(&text, options) else {
        return Vec::new();
    };
    let is_heading = |n: &roxmltree::Node| n.is_element() && matches!(name(*n).as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    let heading_text = |n: roxmltree::Node| collapse(&n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect::<String>()).trim().to_string();
    let nodes: Vec<roxmltree::Node> = doc.descendants().collect();
    let mut out = Vec::new();
    for (ix, node) in nodes.iter().enumerate() {
        let Some(id) = node.attribute("id").filter(|_| node.is_element()) else { continue };
        let heading = if is_heading(node) {
            Some(*node)
        } else {
            node.descendants().find(|d| is_heading(d)).or_else(|| nodes[ix + 1..].iter().copied().find(|d| is_heading(d)))
        };
        if let Some(heading) = heading.map(heading_text).filter(|t| !t.is_empty()) {
            out.push((id.to_string(), heading));
        }
    }
    out
}

pub fn parse_document(text: &str, resolve: ResolveImage) -> Result<Vec<Block>, String> {
    let text = normalize_entities(text);
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(&text, options).map_err(|e| e.to_string())?;
    let body = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name().eq_ignore_ascii_case("body"))
        .unwrap_or_else(|| doc.root_element());
    let mut builder = Builder {
        resolve,
        out: Vec::new(),
        pending: Vec::new(),
    };
    builder.block_children(body);
    builder.flush();
    Ok(builder.out)
}

/// Last resort for content that isn't well-formed XML: strip the tags and
/// keep the text as paragraphs.
pub fn plain_fallback(text: &str) -> Vec<Block> {
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    let lower = text.to_ascii_lowercase();
    let body = lower.find("<body").map_or(text, |at| &text[at..]);
    for c in body.chars() {
        match c {
            '<' => {
                in_tag = true;
                plain.push('\n');
            }
            '>' => in_tag = false,
            c if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain
        .split("\n\n")
        .map(|p| collapse(p).trim().to_string())
        .filter(|p| !p.is_empty())
        .map(|p| Block::paragraph(vec![Span::plain(p)]))
        .collect()
}

fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() && c != '\u{A0}' {
            space = true;
        } else {
            // A leading run counts too (text after an inline element);
            // `tidy` trims the paragraph's outer edges.
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

fn name(node: roxmltree::Node) -> String {
    node.tag_name().name().to_ascii_lowercase()
}

fn is_block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div" | "section" | "article" | "aside" | "header" | "footer" | "main" | "nav"
            | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "ul" | "ol" | "li"
            | "pre" | "table" | "hr" | "figure" | "figcaption" | "dl" | "dt" | "dd" | "center"
            | "address" | "hgroup"
    )
}

struct Builder<'a> {
    resolve: ResolveImage<'a>,
    out: Vec<Block>,
    /// Inline content waiting for its paragraph.
    pending: Vec<Span>,
}

impl Builder<'_> {
    fn flush(&mut self) {
        let spans = tidy(std::mem::take(&mut self.pending));
        if !spans.is_empty() {
            self.out.push(Block::paragraph(spans));
        }
    }

    fn block_children(&mut self, node: roxmltree::Node) {
        for child in node.children() {
            if child.is_text() {
                let text = collapse(child.text().unwrap_or(""));
                if !text.trim().is_empty() || !self.pending.is_empty() {
                    self.pending.push(Span::plain(text));
                }
                continue;
            }
            if !child.is_element() {
                continue;
            }
            let tag = name(child);
            if is_block(&tag) {
                self.flush();
                self.block(child, &tag);
            } else if matches!(tag.as_str(), "script" | "style" | "head" | "title") {
            } else if tag == "img" || tag == "image" {
                self.flush();
                self.image(child);
            } else {
                inline(child, SpanStyle::default(), None, &mut self.pending);
            }
        }
    }

    /// The child blocks of `node` as their own list (quotes, list items).
    fn nested(&mut self, node: roxmltree::Node) -> Vec<Block> {
        let mut inner = Builder {
            resolve: self.resolve,
            out: Vec::new(),
            pending: Vec::new(),
        };
        inner.block_children(node);
        inner.flush();
        inner.out
    }

    fn image(&mut self, node: roxmltree::Node) {
        let src = node
            .attribute("src")
            .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")))
            .or_else(|| node.attribute("href"));
        let alt = node.attribute("alt").unwrap_or("").to_string();
        self.out.push(Block::Image {
            alt,
            resource: src.and_then(|s| (self.resolve)(s)),
        });
    }

    fn block(&mut self, node: roxmltree::Node, tag: &str) {
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let mut spans = Vec::new();
                inline_children(node, SpanStyle::default(), None, &mut spans);
                let spans = tidy(spans);
                if !spans.is_empty() {
                    self.out.push(Block::Heading {
                        level: tag[1..].parse().unwrap_or(1),
                        spans,
                    });
                }
            }
            "blockquote" => {
                let blocks = self.nested(node);
                if !blocks.is_empty() {
                    self.out.push(Block::Quote { blocks });
                }
            }
            "ul" | "ol" => {
                let items: Vec<Vec<Block>> = node
                    .children()
                    .filter(|c| c.is_element() && name(*c) == "li")
                    .map(|li| self.nested(li))
                    .collect();
                if !items.is_empty() {
                    self.out.push(Block::List {
                        ordered: tag == "ol",
                        items,
                    });
                }
            }
            "pre" => {
                let text: String = node
                    .descendants()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .collect();
                self.out.push(Block::Code {
                    language: None,
                    text: text.trim_end_matches('\n').to_string(),
                });
            }
            "hr" => self.out.push(Block::Rule),
            "table" => self.table(node),
            "p" | "dt" | "figcaption" | "address" => {
                // A paragraph may still hold images.
                let mut spans = Vec::new();
                for child in node.children() {
                    if child.is_element() && matches!(name(child).as_str(), "img" | "image") {
                        let done = tidy(std::mem::take(&mut spans));
                        if !done.is_empty() {
                            self.out.push(Block::paragraph(done));
                        }
                        self.image(child);
                    } else if child.is_element() && name(child) == "svg" {
                        for image in child.descendants().filter(|n| n.is_element() && name(*n) == "image") {
                            self.image(image);
                        }
                    } else {
                        inline_node(child, SpanStyle::default(), None, &mut spans);
                    }
                }
                let spans = tidy(spans);
                if !spans.is_empty() {
                    let centered = node
                        .attribute("class")
                        .is_some_and(|c| c.contains("center"))
                        || node
                            .attribute("style")
                            .is_some_and(|s| s.replace(' ', "").contains("text-align:center"));
                    self.out.push(Block::Paragraph {
                        spans,
                        align: if centered {
                            mireader_reader_sdk::Align::Center
                        } else {
                            Default::default()
                        },
                    });
                }
            }
            _ => self.block_children(node),
        }
    }

    fn table(&mut self, node: roxmltree::Node) {
        let mut rows: Vec<Vec<Vec<Span>>> = Vec::new();
        let mut header = false;
        for (ix, tr) in node
            .descendants()
            .filter(|n| n.is_element() && name(*n) == "tr")
            .enumerate()
        {
            let mut row = Vec::new();
            for cell in tr.children().filter(|c| c.is_element() && matches!(name(*c).as_str(), "td" | "th")) {
                if ix == 0 && name(cell) == "th" {
                    header = true;
                }
                let mut spans = Vec::new();
                inline_children(cell, SpanStyle::default(), None, &mut spans);
                row.push(tidy(spans));
            }
            if !row.is_empty() {
                rows.push(row);
            }
        }
        if !rows.is_empty() {
            self.out.push(Block::Table { header, rows });
        }
    }
}

fn inline_children(node: roxmltree::Node, style: SpanStyle, link: Option<&str>, out: &mut Vec<Span>) {
    for child in node.children() {
        inline_node(child, style, link, out);
    }
}

fn inline_node(node: roxmltree::Node, style: SpanStyle, link: Option<&str>, out: &mut Vec<Span>) {
    if node.is_text() {
        out.push(Span {
            text: collapse(node.text().unwrap_or("")),
            style,
            link: link.map(str::to_string),
        });
    } else if node.is_element() {
        inline(node, style, link, out);
    }
}

fn inline(node: roxmltree::Node, mut style: SpanStyle, link: Option<&str>, out: &mut Vec<Span>) {
    let mut own_link: Option<String> = None;
    match name(node).as_str() {
        "b" | "strong" => style.bold = true,
        "i" | "em" | "cite" | "dfn" | "var" => style.italic = true,
        "u" | "ins" => style.underline = true,
        "s" | "strike" | "del" => style.strike = true,
        "code" | "tt" | "kbd" | "samp" => style.code = true,
        // MIREADER (round 16, #80): raised and lowered text.
        "sup" => style.sup = true,
        "sub" => style.sub = true,
        "a" => {
            if let Some(href) = node.attribute("href") {
                if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("mailto:") {
                    own_link = Some(href.to_string());
                } else if !href.trim().is_empty() && !href.contains(':') {
                    // MIREADER (round 15, #46): an in-book link, kept raw;
                    // the book resolves it to `section:N#Heading` (`lib.rs`).
                    own_link = Some(format!("{INBOOK}{href}"));
                }
            }
        }
        "br" => {
            out.push(Span::plain("\n"));
            return;
        }
        "script" | "style" => return,
        _ => {}
    }
    inline_children(node, style, own_link.as_deref().or(link), out);
}

/// Merge equal neighbours, drop empties, trim the paragraph's outer
/// whitespace and spaces around hard breaks.
fn tidy(spans: Vec<Span>) -> Vec<Span> {
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.text.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last) if last.style == span.style && last.link == span.link => last.text.push_str(&span.text),
            _ => merged.push(span),
        }
    }
    for span in &mut merged {
        if span.text.contains('\n') {
            span.text = span.text.replace(" \n", "\n").replace("\n ", "\n");
        }
    }
    if let Some(first) = merged.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = merged.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    merged.retain(|s| !s.text.is_empty());
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use mireader_reader_sdk::model::spans_text;

    fn parse(body: &str) -> Vec<Block> {
        let html = format!(
            "<?xml version=\"1.0\"?><!DOCTYPE html><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title><style>p{{}}</style></head><body>{body}</body></html>"
        );
        parse_document(&html, &|src| Some(format!("OEBPS/{src}"))).unwrap()
    }

    #[test]
    fn maps_blocks_and_inline_styles() {
        let blocks = parse(
            "<h1>Chapter&nbsp;One</h1>\n<p>It was a <em>dark</em> and <b>stormy</b>\n   night &mdash; <a href=\"https://x.org\">link</a>, <a href=\"ch2.xhtml\">inner</a>.</p>\
             <blockquote><p>Quoted.</p></blockquote><ul><li>one</li><li><p>two</p></li></ul>\
             <pre>  code\n  kept</pre><hr/><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td><i>2</i></td></tr></table>\
             <p>Line<br/>break</p><div>loose <span>text</span><p>inner para</p>tail</div><p><img src=\"img/a.png\" alt=\"A map\"/></p>",
        );
        let Block::Heading { level: 1, spans } = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(spans_text(spans), "Chapter\u{A0}One");
        let Block::Paragraph { spans, .. } = &blocks[1] else { panic!() };
        assert_eq!(spans_text(spans), "It was a dark and stormy night \u{2014} link, inner.");
        assert!(spans.iter().any(|s| s.text == "dark" && s.style.italic));
        assert!(spans.iter().any(|s| s.text == "stormy" && s.style.bold));
        assert!(spans.iter().any(|s| s.text == "link" && s.link.as_deref() == Some("https://x.org")));
        // Round 15 (#46): an in-book link comes out raw, for the book to resolve.
        assert!(spans.iter().any(|s| s.text == "inner" && s.link.as_deref() == Some("inbook:ch2.xhtml")));
        assert!(matches!(&blocks[2], Block::Quote { blocks } if blocks.len() == 1));
        assert!(matches!(&blocks[3], Block::List { ordered: false, items } if items.len() == 2));
        assert!(matches!(&blocks[4], Block::Code { text, .. } if text == "  code\n  kept"));
        assert_eq!(blocks[5], Block::Rule);
        assert!(matches!(&blocks[6], Block::Table { header: true, rows } if rows.len() == 2 && rows[1][1][0].style.italic));
        assert!(matches!(&blocks[7], Block::Paragraph { spans, .. } if spans_text(spans) == "Line\nbreak"));
        let texts: Vec<String> = blocks[8..11]
            .iter()
            .map(|b| match b {
                Block::Paragraph { spans, .. } => spans_text(spans),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["loose text", "inner para", "tail"]);
        assert_eq!(
            blocks[11],
            Block::Image {
                alt: "A map".into(),
                resource: Some("OEBPS/img/a.png".into())
            }
        );
        assert_eq!(blocks.len(), 12);
    }

    /// MIREADER (round 16, #80): `<sup>` / `<sub>` keep their rise and drop.
    #[test]
    fn sup_and_sub_are_raised_and_lowered() {
        let blocks = parse("<p>E=mc<sup>2</sup> and H<sub>2</sub>O</p>");
        let Block::Paragraph { spans, .. } = &blocks[0] else { panic!("{blocks:?}") };
        assert!(spans.iter().any(|s| s.text == "2" && s.style.sup && !s.style.sub));
        assert!(spans.iter().any(|s| s.text == "2" && s.style.sub && !s.style.sup));
    }

    #[test]
    fn broken_markup_falls_back_to_text() {
        assert!(parse_document("<html><body><p>unclosed", &|_| None).is_err());
        let blocks = plain_fallback("<html><body><p>one</p>\n\n<p>two <b>bold</p></body>");
        assert_eq!(blocks.len(), 2);
    }
}
