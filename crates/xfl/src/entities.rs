//! Display form of names that carry XML character references.
//!
//! Animate writes numeric references without the closing semicolon in library paths and names, for example
//! `&#060Clip Group&#062` for `<Clip Group>`. Model names keep that text verbatim, because it is the key instances
//! link by and what the writer emits again. Everything a person reads goes through [`display_name`].

use std::borrow::Cow;

/// Longest run of digits taken after `&#` (a code point has at most seven decimal or six hex digits).
const MAX_DIGITS: usize = 8;

/// `name` with its character references decoded, for showing to a person.
///
/// The semicolon after a numeric reference is optional. The five named XML entities are decoded only with their semicolon. Anything that is not a valid reference
/// is left as written. Borrows when there is nothing to decode.
#[must_use]
pub fn display_name(name: &str) -> Cow<'_, str> {
    if !name.contains('&') {
        return Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or_default());
        let tail = rest.get(at..).unwrap_or_default();
        if let Some((c, used)) = decode_one(tail) {
            out.push(c);
            rest = tail.get(used..).unwrap_or_default();
        } else {
            out.push('&');
            rest = tail.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Decodes the reference at the start of `tail` (which begins with `&`): the character and the bytes it spans.
fn decode_one(tail: &str) -> Option<(char, usize)> {
    let body = tail.get(1..)?;
    if let Some(numeric) = body.strip_prefix('#') {
        let (radix, digits_from) = match numeric.strip_prefix(['x', 'X']) {
            Some(_) => (16, 1),
            None => (10, 0),
        };
        let digits = numeric.get(digits_from..)?;
        let count = digits.bytes().take(MAX_DIGITS).take_while(|b| char::from(*b).is_digit(radix)).count();
        if count == 0 {
            return None;
        }
        let value = u32::from_str_radix(digits.get(..count)?, radix).ok()?;
        let c = char::from_u32(value).filter(|c| *c != '\0')?;
        let semicolon = usize::from(digits.get(count..)?.starts_with(';'));
        // `&#` + optional `x` + digits + optional `;`
        Some((c, 2_usize.saturating_add(digits_from).saturating_add(count).saturating_add(semicolon)))
    } else {
        [("amp;", '&'), ("lt;", '<'), ("gt;", '>'), ("quot;", '"'), ("apos;", '\'')]
            .into_iter()
            .find(|(name, _)| body.starts_with(name))
            .map(|(name, c)| (c, name.len().saturating_add(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_references_without_semicolon() {
        assert_eq!(display_name("&#060Clip Group&#062"), "<Clip Group>");
        assert_eq!(display_name("flash0.ai_2/Isolation Mode/&#060Clip Group&#062"), "flash0.ai_2/Isolation Mode/<Clip Group>");
        assert_eq!(display_name("a&#058b"), "a:b");
    }

    #[test]
    fn numeric_references_with_semicolon_and_hex() {
        assert_eq!(display_name("&#60;x&#62;"), "<x>");
        assert_eq!(display_name("&#x3C;x&#x3e;"), "<x>");
        assert_eq!(display_name("&#x3Cx"), "<x");
    }

    #[test]
    fn named_entities_need_their_semicolon() {
        assert_eq!(display_name("Tom &amp; Jerry &lt;1&gt; &quot;q&quot; &apos;"), "Tom & Jerry <1> \"q\" '");
        assert_eq!(display_name("R&D &amp"), "R&D &amp");
    }

    #[test]
    fn invalid_references_stay_as_written() {
        for s in ["&", "&#", "&#x", "&#;", "&#0;", "&#xZ", "&#99999999999", "&#55296;", "&", "a & b", "&&#060"] {
            let shown = display_name(s);
            if s == "&&#060" {
                assert_eq!(shown, "&<");
            } else {
                assert_eq!(shown, s, "{s}");
            }
        }
    }

    #[test]
    fn plain_text_is_borrowed_and_decoding_does_not_touch_the_original() {
        let name = String::from("&#060Clip Group&#062");
        assert!(matches!(display_name("plain"), Cow::Borrowed(_)));
        let _ = display_name(&name);
        assert_eq!(name, "&#060Clip Group&#062");
    }
}
