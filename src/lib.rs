//! MiReader EPUB reader (EPUB 2 and 3) as a flow document: one section per
//! spine item, the book's table of contents (EPUB 3 nav or EPUB 2 NCX) as
//! the outline, images served as resources. DRM-encrypted books are
//! refused with `Encrypted`; font obfuscation alone is fine.
//!
//! Built on `zip` + `roxmltree` (both MIT/Apache) — no GPL code.

pub mod xhtml;

use std::{
    collections::HashMap,
    io::{Read, Seek},
    path::Path,
};

use mireader_reader_sdk::{
    Block, Document, DocumentInfo, ErrorCode, OutlineEntry, PROTOCOL_VERSION, Reader, ReaderError,
    ReaderInfo, SectionInfo, Target, model::blocks_text,
};

pub const ID: &str = "mireader.epub";

#[derive(Default)]
pub struct EpubReader;

impl Reader for EpubReader {
    fn info(&self) -> ReaderInfo {
        ReaderInfo {
            protocol: PROTOCOL_VERSION,
            id: ID.into(),
            name: "EPUB".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            extensions: vec!["epub".into()],
        }
    }

    fn open(&mut self, path: &Path) -> Result<Box<dyn Document>, ReaderError> {
        let file = std::fs::File::open(path)?;
        Ok(Box::new(Epub::open(file)?))
    }
}

struct ManifestItem {
    href: String,
    media_type: String,
}

pub struct Epub<R: Read + Seek> {
    zip: zip::ZipArchive<R>,
    info: DocumentInfo,
    /// Zip paths of the spine documents, in reading order.
    spine: Vec<String>,
    /// Zip path → media type, for resources.
    media_types: HashMap<String, String>,
}

/// Resolve `href` against the directory of `base` (a zip path), dropping
/// any `#fragment` and percent-escapes.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = match base.rfind('/') {
        Some(at) if !href.starts_with('/') => base[..at].split('/').collect(),
        _ => Vec::new(),
    };
    for part in href.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%'
            && ix + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(std::str::from_utf8(&bytes[ix + 1..ix + 3]).unwrap_or("zz"), 16)
        {
            out.push(value);
            ix += 3;
        } else {
            out.push(bytes[ix]);
            ix += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn invalid(message: impl std::fmt::Display) -> ReaderError {
    ReaderError::invalid(format!("not a readable EPUB: {message}"))
}

fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, ReaderError> {
    roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(invalid)
}

fn local<'a>(node: &roxmltree::Node<'a, '_>) -> &'a str {
    node.tag_name().name()
}

impl<R: Read + Seek> Epub<R> {
    pub fn open(reader: R) -> Result<Self, ReaderError> {
        let mut zip = zip::ZipArchive::new(reader).map_err(invalid)?;
        let container = read_text(&mut zip, "META-INF/container.xml")?;
        let opf_path = {
            let doc = parse_xml(&container)?;
            doc.descendants()
                .find(|n| local(n) == "rootfile")
                .and_then(|n| n.attribute("full-path"))
                .map(str::to_string)
                .ok_or_else(|| invalid("no rootfile in container.xml"))?
        };
        // DRM: an encryption.xml that encrypts anything but obfuscated fonts.
        if let Ok(encryption) = read_text(&mut zip, "META-INF/encryption.xml")
            && let Ok(doc) = parse_xml(&encryption)
        {
            let drm = doc.descendants().filter(|n| local(n) == "EncryptionMethod").any(|m| {
                let algorithm = m.attribute("Algorithm").unwrap_or("");
                !algorithm.contains("obfuscation") && !algorithm.ends_with("/fonts")
            });
            if drm {
                return Err(ReaderError::new(ErrorCode::Encrypted, "this book is DRM-protected"));
            }
        }

        let opf = read_text(&mut zip, &opf_path)?;
        let doc = parse_xml(&opf)?;
        let metadata = doc.descendants().find(|n| local(n) == "metadata");
        let meta = |field: &str| {
            metadata
                .and_then(|m| m.children().find(|n| local(n) == field))
                .and_then(|n| n.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };
        let title = meta("title");
        let author = meta("creator");

        let mut items: HashMap<String, ManifestItem> = HashMap::new();
        let mut nav_href = None;
        let mut media_types = HashMap::new();
        for item in doc.descendants().filter(|n| local(n) == "item") {
            let (Some(id), Some(href)) = (item.attribute("id"), item.attribute("href")) else {
                continue;
            };
            let href = resolve(&opf_path, href);
            let media_type = item.attribute("media-type").unwrap_or("").to_string();
            if item.attribute("properties").is_some_and(|p| p.split_whitespace().any(|p| p == "nav")) {
                nav_href = Some(href.clone());
            }
            media_types.insert(href.clone(), media_type.clone());
            items.insert(id.to_string(), ManifestItem { href, media_type });
        }
        let spine_node = doc.descendants().find(|n| local(n) == "spine");
        let ncx_href = spine_node
            .and_then(|s| s.attribute("toc"))
            .and_then(|id| items.get(id))
            .map(|i| i.href.clone())
            .or_else(|| {
                items
                    .values()
                    .find(|i| i.media_type == "application/x-dtbncx+xml")
                    .map(|i| i.href.clone())
            });
        let spine: Vec<String> = spine_node
            .into_iter()
            .flat_map(|s| s.children())
            .filter(|n| local(n) == "itemref")
            .filter(|n| n.attribute("linear") != Some("no"))
            .filter_map(|n| n.attribute("idref").and_then(|id| items.get(id)))
            .filter(|i| i.media_type.contains("html") || i.media_type.is_empty())
            .map(|i| i.href.clone())
            .collect();
        if spine.is_empty() {
            return Err(invalid("the spine lists no documents"));
        }

        // Table of contents → outline over spine sections.
        let section_of = |href: &str| spine.iter().position(|s| s == href).map(|ix| ix as u32);
        let mut outline = Vec::new();
        if let Some(nav) = &nav_href
            && let Ok(text) = read_text(&mut zip, nav)
            && let Ok(nav_doc) = parse_xml(&xhtml_entities(&text))
        {
            let toc = nav_doc
                .descendants()
                .filter(|n| local(n) == "nav")
                .find(|n| {
                    n.attributes()
                        .any(|a| a.name() == "type" && a.value().split_whitespace().any(|v| v == "toc"))
                })
                .or_else(|| nav_doc.descendants().find(|n| local(n) == "nav"));
            if let Some(list) = toc.and_then(|t| t.children().find(|n| local(n) == "ol")) {
                outline = nav_entries(list, nav, &section_of);
            }
        }
        if outline.is_empty()
            && let Some(ncx) = &ncx_href
            && let Ok(text) = read_text(&mut zip, ncx)
            && let Ok(ncx_doc) = parse_xml(&text)
            && let Some(map) = ncx_doc.descendants().find(|n| local(n) == "navMap")
        {
            outline = ncx_entries(map, ncx, &section_of);
        }

        let mut sections = vec![SectionInfo::default(); spine.len()];
        fn titles(entries: &[OutlineEntry], sections: &mut [SectionInfo]) {
            for entry in entries {
                if let Target::Section(ix) = entry.target
                    && let Some(section) = sections.get_mut(ix as usize)
                    && section.title.is_none()
                {
                    section.title = Some(entry.title.clone());
                }
                titles(&entry.children, sections);
            }
        }
        titles(&outline, &mut sections);

        let mut info = DocumentInfo::flow(spine.len());
        info.sections = sections;
        info.outline = outline;
        info.title = title;
        info.author = author;
        Ok(Self {
            zip,
            info,
            spine,
            media_types,
        })
    }

    fn section_blocks(&mut self, section: u32) -> Result<Vec<Block>, ReaderError> {
        let path = self
            .spine
            .get(section as usize)
            .cloned()
            .ok_or_else(|| ReaderError::bad_request(format!("no section {section}")))?;
        let text = read_text(&mut self.zip, &path)?;
        let names: Vec<String> = self.zip.file_names().map(str::to_string).collect();
        let resolve_image = |src: &str| {
            let key = resolve(&path, src);
            names.contains(&key).then_some(key)
        };
        Ok(xhtml::parse_document(&text, &resolve_image).unwrap_or_else(|_| xhtml::plain_fallback(&text)))
    }
}

/// `xhtml::parse_document` handles entities itself; the nav document is
/// parsed directly, so it gets the same treatment.
fn xhtml_entities(text: &str) -> String {
    text.replace("&nbsp;", "&#160;")
}

fn nav_entries(
    list: roxmltree::Node,
    base: &str,
    section_of: &dyn Fn(&str) -> Option<u32>,
) -> Vec<OutlineEntry> {
    let mut entries = Vec::new();
    for li in list.children().filter(|n| local(n) == "li") {
        let link = li.children().find(|n| matches!(local(n), "a" | "span"));
        let title: String = link
            .map(|a| a.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect())
            .unwrap_or_default();
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let children = li
            .children()
            .find(|n| local(n) == "ol")
            .map(|ol| nav_entries(ol, base, section_of))
            .unwrap_or_default();
        let target = link
            .and_then(|a| a.attribute("href"))
            .and_then(|href| section_of(&resolve(base, href)))
            .or_else(|| children.iter().find_map(|c| match c.target {
                Target::Section(s) => Some(s),
                Target::Page(_) => None,
            }));
        if let Some(section) = target
            && !title.is_empty()
        {
            entries.push(OutlineEntry {
                title,
                target: Target::Section(section),
                children,
            });
        } else {
            entries.extend(children);
        }
    }
    entries
}

fn ncx_entries(
    parent: roxmltree::Node,
    base: &str,
    section_of: &dyn Fn(&str) -> Option<u32>,
) -> Vec<OutlineEntry> {
    let mut entries = Vec::new();
    for point in parent.children().filter(|n| local(n) == "navPoint") {
        let title = point
            .children()
            .find(|n| local(n) == "navLabel")
            .and_then(|l| l.descendants().find(|n| local(n) == "text"))
            .and_then(|t| t.text())
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        let children = ncx_entries(point, base, section_of);
        let target = point
            .children()
            .find(|n| local(n) == "content")
            .and_then(|c| c.attribute("src"))
            .and_then(|src| section_of(&resolve(base, src)));
        match target {
            Some(section) if !title.is_empty() => entries.push(OutlineEntry {
                title,
                target: Target::Section(section),
                children,
            }),
            _ => entries.extend(children),
        }
    }
    entries
}

fn read_bytes<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Result<Vec<u8>, ReaderError> {
    let mut file = zip
        .by_name(name)
        .map_err(|_| invalid(format!("missing {name}")))?;
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_text<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Result<String, ReaderError> {
    let bytes = read_bytes(zip, name)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

impl<R: Read + Seek> Document for Epub<R> {
    fn info(&self) -> DocumentInfo {
        self.info.clone()
    }

    fn content(&mut self, section: u32) -> Result<Vec<Block>, ReaderError> {
        self.section_blocks(section)
    }

    fn text(&mut self, index: u32) -> Result<String, ReaderError> {
        Ok(blocks_text(&self.section_blocks(index)?))
    }

    fn resource(&mut self, key: &str) -> Result<(String, Vec<u8>), ReaderError> {
        let bytes = read_bytes(&mut self.zip, key)
            .map_err(|_| ReaderError::bad_request(format!("no resource {key}")))?;
        let mime = self
            .media_types
            .get(key)
            .filter(|m| !m.is_empty())
            .cloned()
            .unwrap_or_else(|| guess_mime(key).to_string());
        Ok((mime, bytes))
    }
}

fn guess_mime(path: &str) -> &'static str {
    match path.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
pub(crate) mod tests;
