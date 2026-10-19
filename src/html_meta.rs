//! Bounded HTML metadata extraction without building or walking a DOM.

use html5ever::tokenizer::{
    BufferQueue, CharacterTokens, EndTag, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
    states::RawKind,
};

use crate::*;

const TITLE_BYTES: usize = 397;
const DESCRIPTION_BYTES: usize = 4096;

#[derive(Debug, Default)]
pub struct HtmlMetadata {
    pub title: Option<String>,
    pub lang: Option<String>,
    pub descr: Option<String>,
}

#[derive(Default)]
struct MetadataState {
    metadata: HtmlMetadata,
    active: bool,
    title_seen: bool,
    pending_space: bool,
    truncated: bool,
    title: String,
}

impl MetadataState {
    fn process_token(&mut self, token: Token) -> TokenSinkResult<()> {
        match token {
            TagToken(tag) if tag.kind == StartTag => match tag.name.as_ref() {
                "html" | "body" => {
                    if let Some(attr) = tag.attrs.iter().find(|a| &*a.name.local == "lang") {
                        self.metadata.lang = bounded_text(&attr.value, 128);
                    }
                }
                "meta" => {
                    let name = ["property", "name", "http-equiv"]
                        .iter()
                        .find_map(|name| tag.attrs.iter().find(|a| &*a.name.local == *name));
                    if name.is_some_and(|a| a.value.eq_ignore_ascii_case("description"))
                        && let Some(attr) = tag.attrs.iter().find(|a| &*a.name.local == "content")
                    {
                        self.metadata.descr = bounded_text(&attr.value, DESCRIPTION_BYTES);
                    }
                }
                "title" => {
                    self.active = !self.title_seen;
                    self.title_seen = true;
                    return TokenSinkResult::RawData(RawKind::Rcdata);
                }
                "script" => return TokenSinkResult::RawData(RawKind::ScriptData),
                "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                    return TokenSinkResult::RawData(RawKind::Rawtext);
                }
                "textarea" => return TokenSinkResult::RawData(RawKind::Rcdata),
                "plaintext" => return TokenSinkResult::Plaintext,
                _ => {}
            },
            TagToken(tag) if tag.kind == EndTag && &*tag.name == "title" => self.active = false,
            CharacterTokens(text) if self.active && !self.truncated => {
                for ch in text.chars() {
                    if ch.is_whitespace() {
                        self.pending_space = !self.title.is_empty();
                    } else if !ch.is_control() {
                        let needed = ch.len_utf8() + usize::from(self.pending_space);
                        if self.title.len() + needed > TITLE_BYTES {
                            self.truncated = true;
                            break;
                        }
                        if self.pending_space {
                            self.title.push(' ');
                            self.pending_space = false;
                        }
                        self.title.push(ch);
                    }
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

#[derive(Default)]
struct MetadataSink(Cell<MetadataState>);

impl TokenSink for MetadataSink {
    type Handle = ();

    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut state = self.0.take();
        let result = state.process_token(token);
        self.0.set(state);
        result
    }
}

fn bounded_text(text: &str, max_bytes: usize) -> Option<String> {
    let text: String = text
        .chars()
        .filter(|ch| !ch.is_control() || ch.is_whitespace())
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = text.floor_char_boundary(max_bytes.min(text.len()));
    (end > 0).then(|| text[..end].to_owned())
}

pub fn extract_metadata(body: &str) -> HtmlMetadata {
    let tokenizer = Tokenizer::new(MetadataSink::default(), Default::default());
    let queue = BufferQueue::default();
    let mut offset = 0;
    while offset < body.len() {
        let end = body.floor_char_boundary((offset + 4096).min(body.len()));
        queue.push_back(body[offset..end].into());
        let _ = tokenizer.feed(&queue);
        offset = end;
    }
    tokenizer.end();
    let mut state = tokenizer.sink.0.into_inner();
    if !state.title.is_empty() {
        if state.truncated {
            state.title.push_str("...");
        }
        state.metadata.title = Some(state.title);
    }
    state.metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_metadata_with_entities_and_ignores_fake_tags() {
        let metadata = extract_metadata(
            "<html lang='fi'><meta name='description' content='Fish &amp; chips'>\
             <!-- <title>fake</title> --><script>'<title>fake</title>'</script>\
             <TITLE> Fish &amp;\n chips </TITLE>",
        );
        assert_eq!(metadata.title.as_deref(), Some("Fish & chips"));
        assert_eq!(metadata.lang.as_deref(), Some("fi"));
        assert_eq!(metadata.descr.as_deref(), Some("Fish & chips"));
    }

    #[test]
    fn handles_unicode_opengraph_and_deep_nesting_without_a_dom() {
        let html = format!(
            "<meta property='og:image' content='x'><meta property='og:imageé' content='x'>{}<title>safe</title>{}",
            "<div>".repeat(10_000),
            "</div>".repeat(10_000)
        );
        assert_eq!(extract_metadata(&html).title.as_deref(), Some("safe"));
    }

    #[test]
    fn bounds_unicode_metadata_and_handles_partial_pages_and_chunk_boundaries() {
        let metadata = extract_metadata(&format!(
            "<html lang='{}'><meta name='description' content='{}'><title>{}",
            "é".repeat(100),
            "é".repeat(3000),
            "é".repeat(1000)
        ));
        assert!(metadata.lang.unwrap().len() <= 128);
        assert!(metadata.descr.unwrap().len() <= DESCRIPTION_BYTES);
        let title = metadata.title.unwrap();
        assert!(title.len() <= 400);
        assert!(title.ends_with("..."));
        for padding in 4080..4100 {
            assert_eq!(
                extract_metadata(&format!("{}<title>A &amp; B</title>", " ".repeat(padding)))
                    .title
                    .as_deref(),
                Some("A & B")
            );
        }
        assert_eq!(extract_metadata("<title>partial").title.as_deref(), Some("partial"));
        assert!(extract_metadata("<body>no title").title.is_none());
    }
}
