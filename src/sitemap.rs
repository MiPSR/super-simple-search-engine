//! Sitemap metadata is not a page-depth hop. Its entries retain the referring
//! page's scope/depth until an actual page candidate is discovered.
use std::io::Read;

const NAMESPACE: &str = "http://www.sitemaps.org/schemas/sitemap/0.9";

#[derive(Debug, PartialEq)]
pub(crate) enum Error {
    Oversized,
    Invalid,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Sitemap {
    pub index: bool,
    pub urls: Vec<String>,
}

pub(crate) fn parse(bytes: &[u8], limit: usize) -> Result<Sitemap, Error> {
    let decoded;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut reader = flate2::read::MultiGzDecoder::new(bytes);
        let mut body = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer).map_err(|_| Error::Invalid)?;
            if count == 0 {
                break;
            }
            if limit != 0 && count > limit.saturating_sub(body.len()) {
                return Err(Error::Oversized);
            }
            body.extend_from_slice(&buffer[..count]);
        }
        decoded = body;
        decoded.as_slice()
    } else {
        bytes
    };
    if limit != 0 && bytes.len() > limit {
        return Err(Error::Oversized);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Invalid)?;
    let doc = roxmltree::Document::parse(text).map_err(|_| Error::Invalid)?;
    let root = doc.root_element();
    let index = match root.tag_name().name() {
        "urlset" => false,
        "sitemapindex" => true,
        _ => return Err(Error::Invalid),
    };
    if !standard(root) {
        return Err(Error::Invalid);
    }
    let entry = if index { "sitemap" } else { "url" };
    let urls = root
        .children()
        .filter(|n| n.is_element() && standard(*n) && n.tag_name().name() == entry)
        .filter_map(|n| {
            n.children()
                .find(|n| n.is_element() && standard(*n) && n.tag_name().name() == "loc")
        })
        .filter_map(|n| n.text())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(Sitemap { index, urls })
}

fn standard(node: roxmltree::Node<'_, '_>) -> bool {
    node.tag_name()
        .namespace()
        .is_none_or(|namespace| namespace == NAMESPACE)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }
    #[test]
    fn namespaces_entities_and_nested_extensions() {
        let bytes = br#"<s:urlset xmlns:s="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1"><s:url><s:loc>https://example.org/a?x=1&amp;y=2</s:loc><image:image><image:loc>https://example.org/image.jpg</image:loc></image:image></s:url></s:urlset>"#;
        assert_eq!(
            parse(bytes, 0).unwrap().urls,
            vec!["https://example.org/a?x=1&y=2"]
        );
        assert_eq!(parse(&gzip(bytes), 0).unwrap(), parse(bytes, 0).unwrap());
        assert_eq!(
            parse(b"<rss><loc>https://example.org/</loc></rss>", 0),
            Err(Error::Invalid)
        );
    }
    #[test]
    fn decoded_limit_crc_and_zero_unlimited() {
        let xml = format!("<urlset>{}</urlset>", " ".repeat(100_000));
        let mut compressed = gzip(xml.as_bytes());
        assert!(compressed.len() < 1024);
        assert_eq!(parse(&compressed, 1024), Err(Error::Oversized));
        assert!(parse(&compressed, 0).is_ok());
        compressed.truncate(compressed.len() - 4);
        assert_eq!(parse(&compressed, 0), Err(Error::Invalid));
    }
}
