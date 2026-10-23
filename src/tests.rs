use std::io::Cursor;

use mireader_reader_sdk::{Block, Document, ErrorCode, Target, model::spans_text};

use super::{Epub, resolve};

#[path = "../tests/fixtures/mod.rs"]
mod fixtures;

fn open(book: fixtures::Book) -> Epub<Cursor<Vec<u8>>> {
    Epub::open(Cursor::new(fixtures::build(book))).unwrap()
}

#[test]
fn epub3_reads_metadata_spine_nav_and_resources() {
    let mut book = open(fixtures::Book { epub2: false, drm: false });
    let info = book.info();
    assert_eq!(info.title.as_deref(), Some("The Test Book"));
    assert_eq!(info.author.as_deref(), Some("Ada Author"));
    assert_eq!(info.sections.len(), 2, "non-linear cover skipped");
    assert_eq!(info.sections[0].title.as_deref(), Some("Chapter One"));
    assert_eq!(info.outline.len(), 2);
    assert_eq!(info.outline[1].target, Target::Section(1));
    assert_eq!(info.outline[1].children[0].title, "Lists");

    let one = book.content(0).unwrap();
    assert!(matches!(&one[0], Block::Heading { level: 1, spans } if spans_text(spans) == "Chapter\u{A0}One"));
    assert!(matches!(&one[1], Block::Paragraph { spans, .. } if spans.iter().any(|s| s.style.italic && s.text == "bright")));
    let Block::Image { alt, resource: Some(key) } = &one[2] else { panic!("{one:?}") };
    assert_eq!(alt, "The map");
    assert_eq!(key, "OEBPS/images/map.png");
    let (mime, bytes) = book.resource(key).unwrap();
    assert_eq!(mime, "image/png");
    assert_eq!(bytes, fixtures::PNG);
    assert!(matches!(&book.content(1).unwrap()[1], Block::List { items, .. } if items.len() == 2));
    assert!(book.text(1).unwrap().contains("second"));
    assert!(book.content(5).is_err());
}

#[test]
fn epub2_uses_the_ncx() {
    let book = open(fixtures::Book { epub2: true, drm: false });
    let info = book.info();
    assert_eq!(info.outline.len(), 2);
    assert_eq!(info.outline[0].title, "Chapter One");
    assert_eq!(info.outline[1].target, Target::Section(1));
}

#[test]
fn drm_books_are_refused() {
    let error = Epub::open(Cursor::new(fixtures::build(fixtures::Book { epub2: false, drm: true })))
        .err()
        .expect("DRM refused");
    assert_eq!(error.code, ErrorCode::Encrypted);
}

#[test]
fn non_epub_zips_and_garbage_are_invalid() {
    assert_eq!(Epub::open(Cursor::new(b"not a zip".to_vec())).err().unwrap().code, ErrorCode::InvalidDocument);
}

#[test]
fn hrefs_resolve_against_their_document() {
    assert_eq!(resolve("OEBPS/text/one.xhtml", "../images/a%20b.png#x"), "OEBPS/images/a b.png");
    assert_eq!(resolve("OEBPS/content.opf", "text/one.xhtml"), "OEBPS/text/one.xhtml");
    assert_eq!(resolve("content.opf", "one.xhtml"), "one.xhtml");
}

/// MIREADER (round 15, #46): in-book links resolve to the spine section, with
/// the heading a fragment leads to; a file outside the spine reads as text;
/// web links stay.
#[test]
fn in_book_links_resolve_to_sections_and_headings() {
    let mut book = open(fixtures::Book { epub2: false, drm: false });
    let one = book.content(0).unwrap();
    let Some(Block::Paragraph { spans, .. }) = one.last() else { panic!("{one:?}") };
    let link = |text: &str| spans.iter().find(|s| s.text == text).and_then(|s| s.link.clone());
    assert_eq!(link("part two").as_deref(), Some("section:1#Part%20Two"));
    assert_eq!(link("chapter two").as_deref(), Some("section:1"));
    assert_eq!(link("the map"), None, "not a section: plain text");
    assert_eq!(link("the web").as_deref(), Some("https://x.org"));
}
