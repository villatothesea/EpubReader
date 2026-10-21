//! End to end: the real EPUB plugin executable driven by the real host.

mod fixtures;

use std::path::Path;

use mireader_reader_host::{
    HostConfig, HostError, ReaderHost,
    sdk::{Block, ErrorCode, markdown},
    stage_package,
};

const EXE: &str = env!("CARGO_BIN_EXE_mireader-reader-epub");

fn setup(dir: &Path) -> ReaderHost {
    let readers = dir.join("readers");
    stage_package(&Path::new(env!("CARGO_MANIFEST_DIR")).join("reader.toml"), Path::new(EXE), &readers).unwrap();
    ReaderHost::new(HostConfig {
        dirs: vec![readers],
        ..HostConfig::default()
    })
}

#[test]
fn reads_a_book_with_its_outline_and_images() {
    let dir = tempfile::tempdir().unwrap();
    let host = setup(dir.path());
    let file = dir.path().join("book.epub");
    std::fs::write(&file, fixtures::build(fixtures::Book { epub2: false, drm: false })).unwrap();

    let doc = host.open(&file).unwrap();
    assert_eq!(doc.reader_name(), "EPUB");
    assert_eq!(doc.info().title.as_deref(), Some("The Test Book"));
    assert_eq!(doc.info().outline.len(), 2);
    let sections = doc.all_content().unwrap();
    let blocks = sections.concat();
    let key = blocks
        .iter()
        .find_map(|b| match b {
            Block::Image { resource: Some(key), .. } => Some(key.clone()),
            _ => None,
        })
        .expect("an image block");
    // Image bytes cross the pipe base64-encoded and come back intact.
    let (mime, bytes) = doc.resource(&key).unwrap();
    assert_eq!(mime, "image/png");
    assert_eq!(bytes, fixtures::PNG);
    let resolver = |k: &str| Some(format!("reader-resource://{k}"));
    let md = markdown::to_markdown(&blocks, &markdown::MarkdownOptions { resource_url: Some(&resolver) });
    assert!(md.contains("# Chapter"), "{md}");
    assert!(md.contains("*bright*"), "{md}");
    assert!(md.contains("reader-resource://OEBPS/images/map.png"), "{md}");
    host.shutdown_all();
}

#[test]
fn drm_is_reported_as_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let host = setup(dir.path());
    let file = dir.path().join("locked.epub");
    std::fs::write(&file, fixtures::build(fixtures::Book { epub2: false, drm: true })).unwrap();
    match host.open(&file) {
        Err(HostError::Reader(error)) => assert_eq!(error.code, ErrorCode::Encrypted),
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("a DRM book must not open"),
    }
    host.shutdown_all();
}
