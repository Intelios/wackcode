//! Turns a fetched body into text the model can read. HTML is cleaned (scripts, forms, inline
//! images and `javascript:` links removed, links made absolute), then Readability (the Firefox
//! reader view, via `dom_smoothie`) finds the article and `htmd` writes it as Markdown. A page
//! Readability can't make sense of falls back to its whole body without navigation and footers.
//! Text formats pass through; anything binary is refused rather than dumped into the context.

use dom_query::Document;
use dom_smoothie::Readability;
use url::Url;

#[derive(Debug)]
pub struct Page {
    pub title: Option<String>,
    /// Markdown for HTML, otherwise the decoded text.
    pub text: String,
}

/// Tags whose content is never useful to the model.
const NOISE_TAGS: &str = "script, style, noscript, template, iframe, object, embed, svg, canvas, form, button, select, dialog";
const FALLBACK_NOISE: &str = "nav, header, footer, aside";
/// Readability's article is used only when it has at least this much text.
const MIN_ARTICLE_CHARS: usize = 200;

pub fn convert(body: &[u8], content_type: &str, url: &Url) -> Result<Page, String> {
    let mime = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let charset = charset_of(content_type);
    if is_html(&mime) || (mime.is_empty() && sniff_html(body)) {
        let html = decode(body, charset.or_else(|| meta_charset(body)).as_deref());
        return Ok(html_to_markdown(&html, url));
    }
    if is_text(&mime) || (mime.is_empty() && !body.iter().take(8_192).any(|byte| *byte == 0)) {
        return Ok(Page { title: None, text: decode(body, charset.as_deref()).trim().to_string() });
    }
    let kind = if mime.is_empty() { "binary content" } else { mime.as_str() };
    Err(format!("{url} is {kind}, which web_fetch can't read. It only reads web pages and text."))
}

fn is_html(mime: &str) -> bool {
    mime == "text/html" || mime == "application/xhtml+xml"
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/ecmascript"
                | "application/x-javascript"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/x-sh"
                | "application/graphql"
        )
}

fn sniff_html(body: &[u8]) -> bool {
    let head: String = body.iter().take(512).map(|byte| *byte as char).collect();
    let head = head.trim_start().to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

fn charset_of(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    let start = lower.find("charset")? + "charset".len();
    let rest = lower[start..].trim_start().strip_prefix('=')?.trim_start().trim_start_matches('"');
    let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!name.is_empty()).then_some(name)
}

/// `<meta charset>` or its http-equiv form, looked for where browsers look: near the top.
fn meta_charset(body: &[u8]) -> Option<String> {
    let head: String = body.iter().take(4_096).map(|byte| *byte as char).collect();
    let lower = head.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find("<meta") {
        let start = from + found;
        let end = lower[start..].find('>').map_or(lower.len(), |end| start + end);
        if let Some(charset) = charset_of(&lower[start..end].replace(['\'', '"'], " ").replace("charset =", "charset=")) {
            return Some(charset);
        }
        from = end;
    }
    None
}

fn decode(body: &[u8], charset: Option<&str>) -> String {
    let encoding = charset.and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes())).unwrap_or(encoding_rs::UTF_8);
    encoding.decode(body).0.into_owned()
}

fn html_to_markdown(html: &str, url: &Url) -> Page {
    let document = Document::from(html);
    clean(&document, url);
    let page_title = Some(document.select("title").text().trim().to_string()).filter(|title| !title.is_empty());
    let cleaned = document.html().to_string();

    let article = Readability::new(cleaned.as_str(), Some(url.as_str()), None).ok().and_then(|mut reader| reader.parse().ok());
    if let Some(article) = article
        && article.text_content.trim().chars().count() >= MIN_ARTICLE_CHARS
    {
        let title = Some(article.title.trim().to_string()).filter(|title| !title.is_empty()).or(page_title);
        return Page { title, text: markdown(&article.content) };
    }

    // Readability changes the document it reads, so the fallback starts from the cleaned page.
    let fresh = Document::from(cleaned.as_str());
    fresh.select(FALLBACK_NOISE).remove();
    let body = fresh.select("body");
    let inner = if body.exists() { body.inner_html().to_string() } else { fresh.html().to_string() };
    Page { title: page_title, text: markdown(&inner) }
}

/// Makes the page safe and useful to read as text: links usable outside the page, no inline
/// `data:` images (base64 would flood the context), no images without alt text, no noise.
fn clean(document: &Document, base: &Url) {
    document.select(NOISE_TAGS).remove();
    for (selector, attribute) in [("a[href]", "href"), ("img[src]", "src")] {
        for element in document.select(selector).iter() {
            let value = element.attr(attribute).map(|value| value.to_string()).unwrap_or_default();
            if value.trim_start().to_ascii_lowercase().starts_with("javascript:") {
                element.remove_attr(attribute);
                continue;
            }
            if let Ok(absolute) = base.join(value.trim()) {
                element.set_attr(attribute, absolute.as_str());
            }
        }
    }
    for image in document.select("img").iter() {
        let src = image.attr("src").map(|value| value.to_string()).unwrap_or_default();
        let alt = image.attr("alt").map(|value| value.trim().to_string()).unwrap_or_default();
        if src.is_empty() || alt.is_empty() || src.starts_with("data:") {
            image.remove();
        }
    }
}

fn markdown(html: &str) -> String {
    let converter = htmd::HtmlToMarkdown::builder()
        .options(htmd::options::Options {
            heading_style: htmd::options::HeadingStyle::Atx,
            code_block_style: htmd::options::CodeBlockStyle::Fenced,
            bullet_list_marker: htmd::options::BulletListMarker::Dash,
            ..Default::default()
        })
        .build();
    tidy(&converter.convert(html).unwrap_or_default())
}

fn tidy(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut blank_run = 0;
    for line in markdown.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn extracts_the_article_as_markdown_with_absolute_links() {
        let article = format!(
            r#"<!doctype html><html><head><title>Guide</title><script>alert(1)</script></head><body>
            <nav><a href="/home">Home</a></nav>
            <article><h1>Install</h1>{}
            <p>See <a href="/docs/more">the docs</a> and <a href="javascript:evil()">this</a>.</p>
            <img src="data:image/png;base64,AAAA" alt="inline"><img src="/logo.png" alt="Logo">
            <pre><code>cargo install</code></pre></article>
            <footer>Copyright</footer></body></html>"#,
            "<p>Run the installer and follow the prompts to finish setting things up properly. </p>".repeat(8)
        );
        let page = convert(article.as_bytes(), "text/html; charset=utf-8", &url("https://example.com/guide/")).unwrap();
        assert_eq!(page.title.as_deref(), Some("Guide"));
        assert!(page.text.contains("Install"), "{}", page.text);
        assert!(page.text.contains("[the docs](https://example.com/docs/more)"), "{}", page.text);
        assert!(page.text.contains("![Logo](https://example.com/logo.png)"), "{}", page.text);
        assert!(page.text.contains("cargo install"));
        assert!(!page.text.contains("alert(1)"));
        assert!(!page.text.contains("base64"));
        assert!(!page.text.contains("javascript:"));
        assert!(!page.text.contains("Copyright"), "{}", page.text);
    }

    #[test]
    fn falls_back_to_the_whole_body_for_pages_readability_rejects() {
        let page = convert(b"<html><body><nav>Menu</nav><p>Short note.</p></body></html>", "text/html", &url("https://example.com/")).unwrap();
        assert_eq!(page.text, "Short note.");
    }

    #[test]
    fn passes_text_through_honours_charsets_sniffs_and_refuses_binaries() {
        let base = url("https://x.test/");
        assert_eq!(convert(br#"{"a":1}"#, "application/json", &base).unwrap().text, r#"{"a":1}"#);
        assert_eq!(convert(&[0x63, 0x61, 0x66, 0xe9], "text/plain; charset=iso-8859-1", &base).unwrap().text, "café");
        assert_eq!(convert(b"<!DOCTYPE html><html><body><p>Sniffed</p></body></html>", "", &base).unwrap().text, "Sniffed");
        let meta = b"<html><head><meta charset=\"windows-1252\"></head><body><p>caf\xe9</p></body></html>";
        assert_eq!(convert(meta, "text/html", &base).unwrap().text, "café");
        assert!(convert(&[1, 0, 2], "", &base).is_err());
        assert!(convert(b"PDF", "application/pdf", &base).unwrap_err().contains("application/pdf"));
    }
}
