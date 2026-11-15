//! Turning raw RFC 822 bytes into what the window shows: header summaries for the list, and a
//! sanitised HTML body (scripts, forms and event handlers removed) for the reader.

use base64::Engine;
use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize)]
pub struct Person {
    pub name: String,
    pub email: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub uid: u32,
    pub from: Person,
    pub to: Vec<Person>,
    pub subject: String,
    /// Unix seconds.
    pub date: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub size: u32,
    /// Scheduled mail: when ToggleMail will send it (unix seconds), else 0.
    pub send_at: i64,
    /// Snoozed mail: when it returns to the inbox (unix seconds), else 0.
    pub snoozed_until: i64,
}

/// Header that holds a scheduled message's send time, and keyword prefix for snoozed mail.
pub const SEND_AT_HEADER: &str = "X-ToggleMail-Send-At";
pub const SNOOZE_KEYWORD: &str = "$SnoozedUntil";

#[derive(Clone, Debug, Serialize)]
pub struct Attachment {
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Full {
    pub uid: u32,
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub reply_to: Vec<Person>,
    pub subject: String,
    pub date: i64,
    pub message_id: String,
    pub references: String,
    /// Safe HTML for the reader frame.
    pub html: String,
    /// Plain text, used for quoting in replies.
    pub text: String,
    /// True when the HTML refers to images on the internet (blocked until the user allows them).
    pub has_remote_images: bool,
    pub attachments: Vec<Attachment>,
}

pub fn people(addr: Option<&Address>) -> Vec<Person> {
    let Some(addr) = addr else { return vec![] };
    addr.iter()
        .map(|a| Person {
            name: a.name().unwrap_or("").to_string(),
            email: a.address().unwrap_or("").to_string(),
        })
        .collect()
}

fn first(addr: Option<&Address>) -> Person {
    people(addr).into_iter().next().unwrap_or(Person { name: String::new(), email: String::new() })
}

pub fn summary(uid: u32, header: &[u8], size: u32, flags: &[String]) -> Summary {
    let msg = MessageParser::default().parse_headers(header);
    let has = |f: &str| flags.iter().any(|x| x.eq_ignore_ascii_case(f));
    Summary {
        uid,
        from: first(msg.as_ref().and_then(|m| m.from())),
        to: people(msg.as_ref().and_then(|m| m.to())),
        subject: msg.as_ref().and_then(|m| m.subject()).unwrap_or("(no subject)").to_string(),
        date: msg.as_ref().and_then(|m| m.date()).map(|d| d.to_timestamp()).unwrap_or(0),
        seen: has("\\Seen"),
        flagged: has("\\Flagged"),
        answered: has("\\Answered"),
        size,
        send_at: msg
            .as_ref()
            .and_then(|m| m.header_raw(SEND_AT_HEADER))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0),
        snoozed_until: flags
            .iter()
            .find_map(|f| f.strip_prefix(SNOOZE_KEYWORD).and_then(|t| t.parse().ok()))
            .unwrap_or(0),
    }
}

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Plain-text mail: kept as written, with web addresses made clickable.
pub(crate) fn text_to_html(text: &str) -> String {
    let linked: Vec<String> = escape(text)
        .split(' ')
        .map(|word| {
            let trimmed = word.trim_end_matches(['.', ',', ')', ';', ':']);
            if (trimmed.starts_with("https://") || trimmed.starts_with("http://")) && !trimmed.contains('"') {
                format!("<a href=\"{trimmed}\">{trimmed}</a>{}", &word[trimmed.len()..])
            } else {
                word.to_string()
            }
        })
        .collect();
    format!(
        "<div style=\"white-space:pre-wrap;word-break:break-word;font-family:inherit\">{}</div>",
        linked.join(" ")
    )
}

pub(crate) fn sanitize(html: &str) -> String {
    let attrs: HashSet<&str> = [
        "style", "class", "align", "valign", "bgcolor", "color", "width", "height", "border",
        "cellpadding", "cellspacing", "dir", "lang", "title", "face", "size",
    ]
    .into_iter()
    .collect();
    let mut b = ammonia::Builder::default();
    b.add_tags(["font", "center", "style", "span", "div"])
        .rm_clean_content_tags(["style"])
        .add_generic_attributes(attrs)
        .add_tag_attributes("img", ["src", "alt", "width", "height"])
        .add_url_schemes(["cid", "data", "mailto"])
        .link_rel(Some("noopener noreferrer"));
    b.clean(html).to_string()
}

pub fn full(uid: u32, raw: &[u8]) -> Option<Full> {
    let msg = MessageParser::default().parse(raw)?;

    let text = msg.body_text(0).map(|t| t.to_string()).unwrap_or_default();
    // An HTML-only message lists the same part as both text and HTML body, so look at the part's
    // real type rather than comparing the two lists.
    let real_html = msg.html_part(0).is_some_and(|p| matches!(p.body, PartType::Html(_)));
    let mut html = match msg.body_html(0) {
        Some(h) if real_html => sanitize(&h),
        _ => text_to_html(&text),
    };

    // Inline images (cid:) become data: URLs so they show without any network access.
    let mut attachments = vec![];
    for (index, part) in msg.attachments().enumerate() {
        let mime = part
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        if let Some(cid) = part.content_id() {
            let needle = format!("cid:{}", cid.trim_matches(['<', '>']));
            if html.contains(&needle) {
                let data = base64::engine::general_purpose::STANDARD.encode(part.contents());
                html = html.replace(&needle, &format!("data:{mime};base64,{data}"));
                continue;
            }
        }
        attachments.push(Attachment {
            index,
            name: part.attachment_name().unwrap_or("attachment").to_string(),
            mime,
            size: part.contents().len(),
        });
    }

    let has_remote_images = html.contains("src=\"http") || html.contains("url(http");
    let references = match msg.references() {
        mail_parser::HeaderValue::Text(t) => format!("<{t}>"),
        mail_parser::HeaderValue::TextList(l) => l.iter().map(|t| format!("<{t}>")).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    };

    Some(Full {
        uid,
        from: first(msg.from()),
        to: people(msg.to()),
        cc: people(msg.cc()),
        reply_to: people(msg.reply_to()),
        subject: msg.subject().unwrap_or("(no subject)").to_string(),
        date: msg.date().map(|d| d.to_timestamp()).unwrap_or(0),
        message_id: msg.message_id().map(|m| format!("<{m}>")).unwrap_or_default(),
        references,
        html,
        text,
        has_remote_images,
        attachments,
    })
}

/// The bytes and file name of attachment `index` (as numbered in `Full::attachments`).
pub fn attachment(raw: &[u8], index: usize) -> Option<(String, Vec<u8>)> {
    let msg = MessageParser::default().parse(raw)?;
    let part = msg.attachments().nth(index)?;
    Some((part.attachment_name().unwrap_or("attachment").to_string(), part.contents().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_only_message_shows_html() {
        let raw = b"From: A <a@x.com>\r\nSubject: Hi\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Hello <a href=\"https://x.com/v\">Verify</a></p>";
        let m = full(1, raw).unwrap();
        assert!(m.html.contains("<a href=\"https://x.com/v\""), "{}", m.html);
        assert!(!m.html.contains("pre-wrap"));
    }

    #[test]
    fn alternative_prefers_html() {
        let raw = b"From: a@x.com\r\nSubject: Hi\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nplain\r\n--b\r\nContent-Type: text/html\r\n\r\n<b>rich</b>\r\n--b--\r\n";
        let m = full(1, raw).unwrap();
        assert!(m.html.contains("<b>rich</b>"), "{}", m.html);
        assert_eq!(m.text.trim(), "plain");
    }

    #[test]
    fn plain_text_links_are_clickable() {
        let raw = b"From: a@x.com\r\nSubject: Hi\r\nContent-Type: text/plain\r\n\r\nSee https://example.com/a?b=1. Thanks";
        let m = full(1, raw).unwrap();
        assert!(m.html.contains("<a href=\"https://example.com/a?b=1\">"), "{}", m.html);
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    /// One message with a PDF, a non-ASCII file name (RFC 2231), an inline image used in the HTML,
    /// a second image as a normal attachment, and a forwarded email.
    const RAW: &str = concat!(
        "From: A <a@x.com>\r\nTo: b@y.com\r\nSubject: files\r\nMIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n",
        "--m\r\nContent-Type: multipart/related; boundary=\"r\"\r\n\r\n",
        "--r\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>See <img src=\"cid:logo@x\"></p>\r\n",
        "--r\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n",
        "--r--\r\n",
        "--m\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n",
        "--m\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename*=utf-8''%E0%AE%95%E0%AF%8B%E0%AE%AA%E0%AF%8D%E0%AE%AA%E0%AF%81.txt\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n",
        "--m\r\nContent-Type: image/jpeg; name=\"photo.jpg\"\r\nContent-Disposition: attachment; filename=\"photo.jpg\"\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\n/9j/4AAQ\r\n",
        "--m\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=\"forwarded.eml\"\r\n\r\n",
        "From: c@z.com\r\nSubject: inner\r\n\r\ninner body\r\n",
        "--m--\r\n"
    );

    #[test]
    fn every_attachment_is_listed_and_saved() {
        let m = full(1, RAW.as_bytes()).unwrap();
        let names: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"report.pdf"), "{names:?}");
        assert!(names.contains(&"கோப்பு.txt"), "{names:?}");
        assert!(names.contains(&"photo.jpg"), "{names:?}");
        assert!(names.iter().any(|n| n.ends_with(".eml")), "{names:?}");
        // The inline logo is shown in the message, not listed.
        assert!(m.html.contains("data:image/png;base64,"), "{}", m.html);
        assert_eq!(m.attachments.len(), 4, "{names:?}");
        for a in &m.attachments {
            let (name, bytes) = attachment(RAW.as_bytes(), a.index).unwrap();
            assert_eq!(name, a.name);
            assert!(!bytes.is_empty(), "{name} is empty");
        }
        let (_, pdf) = attachment(RAW.as_bytes(), m.attachments.iter().find(|a| a.name == "report.pdf").unwrap().index).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }
}
