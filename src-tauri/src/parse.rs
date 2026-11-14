//! Turning raw RFC 822 bytes into what the window shows: header summaries for the list, and a
//! sanitised HTML body (scripts, forms and event handlers removed) for the reader.

use base64::Engine;
use mail_parser::{Address, MessageParser, MimeHeaders};
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
}

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
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn text_to_html(text: &str) -> String {
    format!(
        "<div style=\"white-space:pre-wrap;font-family:inherit\">{}</div>",
        escape(text)
    )
}

fn sanitize(html: &str) -> String {
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
    let mut html = match msg.body_html(0) {
        Some(h) if msg.html_body_count() > 0 && !msg.html_body.is_empty() && msg.text_body != msg.html_body => sanitize(&h),
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
