//! Test books built in code (no binary fixtures in the repo).

use std::io::Write;

use zip::{CompressionMethod, write::SimpleFileOptions};

pub struct Book {
    /// EPUB 2 (NCX) instead of EPUB 3 (nav document).
    pub epub2: bool,
    /// Add a DRM encryption.xml.
    pub drm: bool,
}

const CHAPTER_ONE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head>
<body><h1>Chapter&nbsp;One</h1><p>It was a <em>bright</em> cold day.</p>
<p><img src="../images/map.png" alt="The map"/></p></body></html>"#;

const CHAPTER_TWO: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Two</title></head>
<body><h2 id="s">Part Two</h2><ul><li>first</li><li>second</li></ul></body></html>"#;

const NAV: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head>
<body><nav epub:type="toc"><ol>
<li><a href="text/one.xhtml">Chapter One</a></li>
<li><a href="text/two.xhtml#s">Part Two</a><ol><li><a href="text/two.xhtml#s">Lists</a></li></ol></li>
</ol></nav></body></html>"#;

const NCX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap>
<navPoint id="n1" playOrder="1"><navLabel><text>Chapter One</text></navLabel><content src="text/one.xhtml"/></navPoint>
<navPoint id="n2" playOrder="2"><navLabel><text>Part Two</text></navLabel><content src="text/two.xhtml"/></navPoint>
</navMap></ncx>"#;

/// A 1×1 PNG.
pub const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

pub fn build(book: Book) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut put = |name: &str, bytes: &[u8], options: SimpleFileOptions| {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    };
    put("mimetype", b"application/epub+zip", stored);
    put(
        "META-INF/container.xml",
        br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        deflated,
    );
    if book.drm {
        put(
            "META-INF/encryption.xml",
            br#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></enc:EncryptedData></encryption>"#,
            deflated,
        );
    }
    let toc_item = if book.epub2 {
        r#"<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#
    } else {
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#
    };
    let spine_toc = if book.epub2 { r#" toc="ncx""# } else { "" };
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>The Test Book</dc:title><dc:creator>Ada Author</dc:creator></metadata>
<manifest>{toc_item}
<item id="one" href="text/one.xhtml" media-type="application/xhtml+xml"/>
<item id="two" href="text/two.xhtml" media-type="application/xhtml+xml"/>
<item id="cover" href="text/cover.xhtml" media-type="application/xhtml+xml"/>
<item id="map" href="images/map.png" media-type="image/png"/>
</manifest>
<spine{spine_toc}><itemref idref="cover" linear="no"/><itemref idref="one"/><itemref idref="two"/></spine>
</package>"#,
        version = if book.epub2 { "2.0" } else { "3.0" },
    );
    put("OEBPS/content.opf", opf.as_bytes(), deflated);
    if book.epub2 {
        put("OEBPS/toc.ncx", NCX.as_bytes(), deflated);
    } else {
        put("OEBPS/nav.xhtml", NAV.as_bytes(), deflated);
    }
    put("OEBPS/text/cover.xhtml", b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>cover</p></body></html>", deflated);
    put("OEBPS/text/one.xhtml", CHAPTER_ONE.as_bytes(), deflated);
    put("OEBPS/text/two.xhtml", CHAPTER_TWO.as_bytes(), deflated);
    put("OEBPS/images/map.png", PNG, stored);
    zip.finish().unwrap().into_inner()
}
