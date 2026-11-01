//! Translations of what AAE shows and says.
//!
//! English stays the source everywhere: the code writes its text in English,
//! and [`tr`] turns finished English text into the user's language on its way
//! out, in the apps and in `aae`. A catalog, such as `translations/fr.po`,
//! pairs each English text with its translation. Text with parts filled in,
//! such as "{device} is ready.", is found as a pattern, and its parts are put
//! in their place in the translation as they are: names of devices, apps,
//! files and snapshots never change. Only a part the catalog marks as a
//! message of its own, such as `{why}` or `{e}`, is translated too.
//! Text with no translation stays in English.
//!
//! A part holds a message when what fills it in the code is named as one,
//! such as `e`, `why`, `error` or `status.detail`: translations/extract.py
//! finds these and marks them in the catalog ("#. messages: e"). For a line
//! that mixes AAE's words with names or the device's own text, translate its
//! format with [`tr_format`], which keeps the values as they are. Text that
//! isn't AAE's, such as what the device shows or JSON for programs, isn't
//! passed to either.
//!
//! The MCP server stays in English: AI agents read it.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

/// The languages with a catalog, by their two-letter code.
static CATALOGS: &[(&str, &str)] = &[(
    "fr",
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../translations/fr.po")),
)];

static PARSED: LazyLock<HashMap<&'static str, Catalog>> = LazyLock::new(|| {
    CATALOGS
        .iter()
        .map(|(code, po)| (*code, Catalog::parse(po)))
        .collect()
});

static CURRENT: RwLock<Option<&'static str>> = RwLock::new(None);

/// Uses the language of a tag such as "fr", "fr-FR" or "fr_FR.UTF-8", or
/// English when AAE has no catalog for it. Returns whether it has one.
pub fn set_language(tag: &str) -> bool {
    let code = tag
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let found = CATALOGS.iter().find(|(c, _)| *c == code).map(|(c, _)| *c);
    if let Ok(mut current) = CURRENT.write() {
        *current = found;
    }
    found.is_some()
}

/// The language `aae` uses: AAE_LANG, then the usual LC_ALL, LC_MESSAGES
/// and LANG.
pub fn language_from_env() -> Option<String> {
    ["AAE_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
}

/// The two-letter code of the language in use, if not English.
pub fn language() -> Option<&'static str> {
    CURRENT.read().ok().and_then(|c| *c)
}

/// The English text in the language in use.
pub fn tr(text: &str) -> String {
    match language().and_then(|code| PARSED.get(code)) {
        Some(catalog) => catalog.translate(text),
        None => text.to_string(),
    }
}

/// A format string such as "{}: {} ({})." in the language in use, with its
/// parts filled in, in order, by values kept exactly as they are: names, or
/// text already translated with [`tr`]. For a line that mixes AAE's words
/// with a device's, such as an app's label and whether it's on.
pub fn tr_format(format: &str, values: &[&str]) -> String {
    tr_format_from(language().and_then(|code| PARSED.get(code)), format, values)
}

fn tr_format_from(catalog: Option<&Catalog>, format: &str, values: &[&str]) -> String {
    // Looked up without the spaces around it, which are kept, as in tr.
    let trimmed = format.trim();
    let start = format.len() - format.trim_start().len();
    let translated = catalog
        .and_then(|catalog| catalog.formats.get(trimmed))
        .map(|t| format!("{}{t}{}", &format[..start], &format[start + trimmed.len()..]));
    let mut by_key = HashMap::new();
    let mut next = values.iter();
    for part in parts(format) {
        if let Part::Slot(key) = part
            && !by_key.contains_key(&key)
        {
            by_key.insert(key, *next.next().unwrap_or(&""));
        }
    }
    parts(translated.as_deref().unwrap_or(format))
        .into_iter()
        .map(|part| match part {
            Part::Text(text) => text,
            Part::Slot(key) => by_key.get(&key).copied().unwrap_or("").to_string(),
        })
        .collect()
}

/// As `tr`, for a word with more than one meaning in English, such as
/// "Watch" (a button) and "Watch" (a device): the catalog's entry with this
/// context (msgctxt), or else the text's usual translation.
pub fn tr_in(context: &str, text: &str) -> String {
    let found = language()
        .and_then(|code| PARSED.get(code))
        .and_then(|catalog| catalog.exact.get(&format!("{context}\u{4}{text}")));
    match found {
        Some(translated) => translated.clone(),
        None => tr(text),
    }
}

/// The catalog of a language as an Apple Localizable.strings file, for the
/// labels written in the Mac app's views, such as `Button("Cold Boot")`,
/// which SwiftUI translates itself. macos/build.sh writes it with
/// `aae localizable-strings`. Entries without a translation are left out, so
/// they stay in English.
pub fn localizable_strings(code: &str) -> Option<String> {
    let (_, po) = CATALOGS.iter().find(|(c, _)| *c == code)?;
    let mut out = String::new();
    let mut seen = HashSet::new();
    for (english, translated, _) in po_entries(po) {
        // SwiftUI has no contexts; those entries are for tr(_:context:).
        if english.is_empty() || translated.is_empty() || english.contains('\u{4}') {
            continue;
        }
        for (key, value) in swiftui_keys(&english, &translated) {
            if seen.insert(key.clone()) {
                out.push_str(&format!("{} = {};\n", strings_quote(&key), strings_quote(&value)));
            }
        }
    }
    Some(out)
}

/// The parts filled in, `{name}`, `{}` or `{0}`, as SwiftUI keys see them:
/// each one's place in the text and its key, a name, or "#" and its
/// position among the unnamed ones ("#0" for `{0}` too).
fn swift_slots(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let bytes = text.as_bytes();
    let mut slots = Vec::new();
    let mut unnamed = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' || (i > 0 && bytes[i - 1] == b'{') {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        let name = &text[i + 1..j];
        if j < bytes.len() && bytes[j] == b':' {
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
        }
        if j >= bytes.len() || bytes[j] != b'}' || bytes.get(j + 1) == Some(&b'}') {
            i += 1;
            continue;
        }
        let key = if name.is_empty() {
            unnamed += 1;
            format!("#{}", unnamed - 1)
        } else if name.bytes().all(|b| b.is_ascii_digit()) {
            format!("#{name}")
        } else {
            name.to_string()
        };
        slots.push((i..j + 1, key));
        i = j + 1;
    }
    slots
}

/// The keys SwiftUI looks up for a label with parts, such as
/// Text("Apps on \(name)"): "Apps on %@" for text, "%lld" for a number, with
/// the translation's parts by position.
fn swiftui_keys(english: &str, translated: &str) -> Vec<(String, String)> {
    if swift_slots(english).is_empty() {
        return vec![(english.to_string(), translated.to_string())];
    }
    // In a format, a percent sign of the text itself is written %%.
    let english = english.replace('%', "%%");
    let translated = translated.replace('%', "%%");
    let english_slots = swift_slots(&english);
    if english_slots.len() > 3 {
        return Vec::new();
    }
    let mut position: HashMap<&str, usize> = HashMap::new();
    for (i, (_, key)) in english_slots.iter().enumerate() {
        position.entry(key.as_str()).or_insert(i + 1);
    }
    let translated_slots = swift_slots(&translated);
    if translated_slots.iter().any(|(_, key)| !position.contains_key(key.as_str())) {
        return Vec::new();
    }
    let replace = |text: &str, slots: &[(std::ops::Range<usize>, String)], with: &dyn Fn(&str) -> String| {
        let mut out = String::new();
        let mut last = 0;
        for (range, key) in slots {
            out.push_str(&text[last..range.start]);
            out.push_str(&with(key));
            last = range.end;
        }
        out.push_str(&text[last..]);
        out
    };
    let mut keys = Vec::new();
    // Each part as text or as a number, the last part changing first.
    for kinds in 0..1usize << english_slots.len() {
        let mut kind_of: HashMap<&str, &str> = HashMap::new();
        for (i, (_, key)) in english_slots.iter().enumerate() {
            let bit = english_slots.len() - 1 - i;
            kind_of.entry(key.as_str()).or_insert(if kinds >> bit & 1 == 0 { "@" } else { "lld" });
        }
        let key = replace(&english, &english_slots, &|k| format!("%{}", kind_of[k]));
        let value = replace(&translated, &translated_slots, &|k| format!("%{}${}", position[k], kind_of[k]));
        keys.push((key, value));
    }
    keys
}

fn strings_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

/// One English text and its translation, as parts.
struct Template {
    english: Vec<Part>,
    translated: Vec<Part>,
    /// The length of its fixed text, to try the most specific first.
    fixed: usize,
    /// Its fixed text ends a sentence or introduces one, as in "Couldn't
    /// start {device}: {why}", so a part may hold whole sentences. A pattern
    /// such as "{} and {}" joins names, not sentences.
    holds_sentences: bool,
    /// The parts that hold a message of their own, translated too. Others,
    /// such as names, are kept as they are.
    messages: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    /// A part filled in, by name ("device") or position ("0", "1"…).
    Slot(String),
}

#[derive(Default)]
struct Catalog {
    exact: HashMap<String, String>,
    templates: Vec<Template>,
    /// Words that start a text as a label, as "Warning" in "Warning: {}".
    /// Only these are translated before a colon, so a device named "Phone"
    /// keeps its name in "Phone: Android 17, Phone".
    labels: HashSet<String>,
    /// The most sentences and lines in one English text, the longest run
    /// worth looking up.
    longest: usize,
    /// Each English text with parts and its translation, as written, for
    /// [`tr_format`].
    formats: HashMap<String, String>,
}

/// Text longer than this, or with more sentences and lines than
/// MAX_PIECES, isn't AAE's message but something it shows, such as an
/// inspector dump: it's looked up only as a whole.
const MAX_LEN: usize = 16 * 1024;
const MAX_PIECES: usize = 200;

impl Catalog {
    fn parse(po: &str) -> Catalog {
        let mut catalog = Catalog::default();
        for (english, translated, messages) in po_entries(po) {
            // Text is looked up without the spaces around it, which are
            // kept as they are (see translate_whole).
            let (english, translated) = (english.trim().to_string(), translated.trim().to_string());
            // A translation that is the English text adds nothing, and as a
            // pattern it could hide a better one.
            if english.is_empty() || translated.is_empty() || translated == english {
                continue;
            }
            if !english.contains('\u{4}') {
                catalog.longest = catalog.longest.max(split_sentences(&english).len());
                if let Some((label, after)) = english.split_once(':')
                    && (after.is_empty() || after.starts_with(' '))
                    && !label.is_empty()
                    && label.len() <= 40
                    && !label.contains(['{', '%', '.', '\n'])
                {
                    catalog.labels.insert(label.to_string());
                }
            }
            let english_parts = parts(&english);
            if !english_parts.iter().any(|p| matches!(p, Part::Slot(_))) {
                catalog.exact.insert(english, translated);
                continue;
            }
            catalog.formats.insert(english.clone(), translated.clone());
            // Two parts filled in side by side can't be told apart, and a
            // pattern with no words of its own, such as "{kind}: {title}",
            // would match almost any text.
            if english_parts
                .windows(2)
                .any(|w| matches!(w, [Part::Slot(_), Part::Slot(_)]))
                || !english_parts.iter().any(|p| match p {
                    Part::Text(t) => t.chars().any(char::is_alphabetic),
                    Part::Slot(_) => false,
                })
            {
                continue;
            }
            let fixed = english_parts
                .iter()
                .map(|p| match p {
                    Part::Text(t) => t.len(),
                    Part::Slot(_) => 0,
                })
                .sum();
            let holds_sentences = english_parts.iter().any(|p| match p {
                Part::Text(t) => t.contains(['.', ':', '?', '!']),
                Part::Slot(_) => false,
            });
            catalog.templates.push(Template {
                english: english_parts,
                translated: parts(&translated),
                fixed,
                holds_sentences,
                messages: messages.into_iter().collect(),
            });
        }
        catalog.templates.sort_by_key(|t| std::cmp::Reverse(t.fixed));
        catalog
    }

    fn translate(&self, text: &str) -> String {
        if text.len() > MAX_LEN {
            return text.to_string();
        }
        match self.translate_any(text, 0) {
            Some(found) => found,
            None => {
                note_missing(text);
                text.to_string()
            }
        }
    }

    /// The text as a whole, or else sentence by sentence and line by line,
    /// as when messages are joined. None if AAE knows none of it.
    fn translate_any(&self, text: &str, depth: u8) -> Option<String> {
        if let Some(found) = self.translate_whole(text, depth) {
            return Some(found);
        }
        let pieces = split_sentences(text);
        let count = pieces.len();
        if !(2..=MAX_PIECES).contains(&count) {
            return None;
        }
        // From the left, the longest run of sentences AAE knows as one
        // text, as a message of two sentences after a label. No run is
        // longer than the catalog's longest text, so this grows with the
        // text's length, not its square.
        let longest = self.longest.max(1);
        let mut out = String::new();
        let mut any = false;
        let mut start = 0;
        while start < count {
            let mut found = None;
            for end in (start + 1..=count.min(start + longest)).rev() {
                if start == 0 && end == count {
                    continue; // The whole text, tried above.
                }
                let run: String = pieces[start..end]
                    .iter()
                    .enumerate()
                    .map(|(i, (piece, gap))| {
                        if start + i + 1 == end { piece.to_string() } else { format!("{piece}{gap}") }
                    })
                    .collect();
                if !run.chars().any(char::is_alphabetic) {
                    continue;
                }
                if let Some(translated) = self.translate_whole(&run, depth) {
                    found = Some((translated, end));
                    break;
                }
            }
            match found {
                Some((translated, end)) => {
                    any = true;
                    out.push_str(&translated);
                    out.push_str(pieces[end - 1].1);
                    start = end;
                }
                None => {
                    let (piece, gap) = pieces[start];
                    if depth == 0 {
                        note_missing(piece);
                    }
                    out.push_str(piece);
                    out.push_str(gap);
                    start += 1;
                }
            }
        }
        any.then_some(out)
    }

    /// The whole text translated, keeping the spaces around it.
    fn translate_whole(&self, text: &str, depth: u8) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() || !trimmed.chars().any(char::is_alphabetic) {
            return Some(text.to_string());
        }
        let start = text.len() - text.trim_start().len();
        let end = start + trimmed.len();
        let translated = self
            .translate_trimmed(trimmed, depth)
            .or_else(|| self.translate_list(trimmed, depth))?;
        Some(format!("{}{}{}", &text[..start], translated, &text[end..]))
    }

    /// A list such as "16 KB pages, With Google services, Phone": each item
    /// AAE knows translated, the others kept. None if it knows none.
    fn translate_list(&self, text: &str, depth: u8) -> Option<String> {
        let (body, end) = match text.strip_suffix('.') {
            Some(body) => (body, "."),
            None => (text, ""),
        };
        // Several sentences are translated one by one first (see translate).
        if body.contains(". ") || body.contains('\n') {
            return None;
        }
        for separator in ["; ", ", "] {
            if !body.contains(separator) {
                continue;
            }
            let mut any = false;
            let items: Vec<String> = body
                .split(separator)
                .map(|item| match self.translate_trimmed(item.trim(), depth + 1) {
                    Some(t) => {
                        any = true;
                        t
                    }
                    None => item.to_string(),
                })
                .collect();
            if any {
                let separator = if separator == "; " { "\u{a0}; " } else { separator };
                return Some(items.join(separator) + end);
            }
        }
        None
    }

    fn translate_trimmed(&self, text: &str, depth: u8) -> Option<String> {
        // Exact text first, in each form, then patterns, so "Android
        // versions." is "Android versions" with a period, not "Android {}".
        self.forms(text, &|form| self.exact.get(form).cloned())
            .or_else(|| self.forms(text, &|form| self.pattern(form, depth)))
            .or_else(|| {
                // A value shown with a capital, such as "Full speed" for
                // "full speed".
                let mut chars = text.chars();
                let first = chars.next().filter(|c| c.is_uppercase())?;
                let lower = first.to_lowercase().collect::<String>() + chars.as_str();
                let found = self.forms(&lower, &|form| self.exact.get(form).cloned())?;
                let mut chars = found.chars();
                let first = chars.next()?;
                Some(first.to_uppercase().collect::<String>() + chars.as_str())
            })
            .or_else(|| {
                // A one-word label the same in French, such as "OK:", still
                // takes French spacing.
                let label = text.strip_suffix(':')?;
                (!label.is_empty() && label.chars().all(char::is_alphanumeric))
                    .then(|| format!("{label}\u{a0}:"))
            })
    }

    /// The text as it is; a label such as "Warning:" as its word with French
    /// spacing; and the text with or without a final period, as when a
    /// step's name is said as a sentence.
    fn forms(&self, text: &str, find: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        if let Some(found) = find(text) {
            return Some(found);
        }
        if let Some(label) = text.strip_suffix(':')
            && self.labels.contains(label)
            && let Some(found) = find(label)
        {
            return Some(format!("{found}\u{a0}:"));
        }
        match text.strip_suffix('.') {
            Some(bare) if !bare.ends_with('.') => {
                find(bare).map(|t| if t.ends_with(['.', '!', '?', '…']) { t } else { t + "." })
            }
            Some(_) => None,
            None => find(&format!("{text}."))
                .map(|t| t.strip_suffix('.').map(str::to_string).unwrap_or(t)),
        }
    }

    fn pattern(&self, text: &str, depth: u8) -> Option<String> {
        for template in &self.templates {
            let Some(values) = fill(&template.english, text) else {
                continue;
            };
            if !template.holds_sentences
                && values
                    .values()
                    .any(|v| [". ", "? ", "! ", "\n"].iter().any(|end| v.contains(end)))
            {
                continue;
            }
            let mut out = String::new();
            for part in &template.translated {
                match part {
                    Part::Text(t) => out.push_str(t),
                    Part::Slot(name) => {
                        let value = values.get(name).map(String::as_str).unwrap_or("");
                        // A part may be a message of its own, such as why
                        // something failed. Names stay as they are.
                        let value = if depth < 2 && template.messages.contains(name) {
                            self.translate_any(value, depth + 1)
                                .unwrap_or_else(|| value.to_string())
                        } else {
                            value.to_string()
                        };
                        out.push_str(&value);
                    }
                }
            }
            return Some(out);
        }
        None
    }
}

/// The values of a template's parts in a text, if the text is that template.
fn fill(template: &[Part], text: &str) -> Option<HashMap<String, String>> {
    let mut values = HashMap::new();
    let mut rest = text;
    let mut parts = template.iter().peekable();
    // The last fixed text must end the text.
    let suffix = match template.last() {
        Some(Part::Text(t)) => t.as_str(),
        _ => "",
    };
    while let Some(part) = parts.next() {
        match part {
            Part::Text(t) => {
                if parts.peek().is_none() {
                    // Already checked as the suffix.
                    if rest != t.as_str() {
                        return None;
                    }
                    rest = "";
                } else {
                    rest = rest.strip_prefix(t.as_str())?;
                }
            }
            Part::Slot(name) => {
                let value_len = match parts.peek() {
                    None => rest.len(),
                    Some(Part::Text(next)) => {
                        if parts.clone().nth(1).is_none() {
                            // The next text ends the template.
                            rest.len().checked_sub(suffix.len())
                                .filter(|_| rest.ends_with(suffix))?
                        } else {
                            rest.find(next.as_str())?
                        }
                    }
                    Some(Part::Slot(_)) => return None,
                };
                // Only a part after some fixed text may be empty, as in
                // "It is running{}." with nothing to add.
                if value_len == 0 && rest.len() == text.len() {
                    return None;
                }
                // A part that starts the text is a name or a value, never
                // the end of an earlier sentence: "{} installed." isn't
                // "OK: Android versions. 1 installed."
                let value = &rest[..value_len];
                if rest.len() == text.len()
                    && [". ", "? ", "! ", ": ", "\n"].iter().any(|end| value.contains(end))
                {
                    return None;
                }
                values.insert(name.clone(), value.to_string());
                rest = &rest[value_len..];
            }
        }
    }
    rest.is_empty().then_some(values)
}

/// A text's fixed parts and filled-in parts. `{name}` and `{}` are Rust's,
/// `%@`, `%lld` and `%1$@` Swift's. Unnamed ones are numbered in order.
fn parts(text: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut fixed = String::new();
    let mut position = 0;
    let mut chars = text.char_indices().peekable();
    let push_fixed = |parts: &mut Vec<Part>, fixed: &mut String| {
        if !fixed.is_empty() {
            parts.push(Part::Text(std::mem::take(fixed)));
        }
    };
    while let Some((i, c)) = chars.next() {
        match c {
            '{' if text[i + 1..].starts_with('{') => {
                fixed.push('{');
                chars.next();
            }
            '}' if text[i + 1..].starts_with('}') => {
                fixed.push('}');
                chars.next();
            }
            '{' => {
                let Some(close) = text[i..].find('}') else {
                    fixed.push(c);
                    continue;
                };
                let inside = &text[i + 1..i + close];
                let name = inside.split(':').next().unwrap_or("");
                let ok = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !ok {
                    fixed.push(c);
                    continue;
                }
                push_fixed(&mut parts, &mut fixed);
                let key = if name.is_empty() {
                    position += 1;
                    (position - 1).to_string()
                } else {
                    name.to_string()
                };
                parts.push(Part::Slot(key));
                while let Some((j, _)) = chars.peek() {
                    if *j > i + close {
                        break;
                    }
                    chars.next();
                }
            }
            '%' => {
                let spec = swift_format(&text[i..]);
                match spec {
                    Some((len, index)) => {
                        push_fixed(&mut parts, &mut fixed);
                        let key = match index {
                            Some(n) => (n - 1).to_string(),
                            None => {
                                position += 1;
                                (position - 1).to_string()
                            }
                        };
                        parts.push(Part::Slot(key));
                        while let Some((j, _)) = chars.peek() {
                            if *j >= i + len {
                                break;
                            }
                            chars.next();
                        }
                    }
                    None => {
                        if text[i + 1..].starts_with('%') {
                            chars.next();
                        }
                        fixed.push('%');
                    }
                }
            }
            _ => fixed.push(c),
        }
    }
    push_fixed(&mut parts, &mut fixed);
    parts
}

/// A Swift format such as `%@`, `%lld` or `%1$@` at the start of a text: its
/// length and its explicit position, if any.
fn swift_format(text: &str) -> Option<(usize, Option<usize>)> {
    let rest = &text[1..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let (index, after) = match rest[digits.len()..].strip_prefix('$') {
        Some(after) if !digits.is_empty() => (digits.parse().ok(), after),
        _ => (None, rest),
    };
    for spec in ["@", "lld", "llu", "ld", "lu", "d", "u", "f", ".1f", ".2f", ".0f"] {
        if after.starts_with(spec) {
            let len = 1 + (rest.len() - after.len()) + spec.len();
            return Some((len, index));
        }
    }
    None
}

/// Sentences and lines, each with the space after it.
fn split_sentences(text: &str) -> Vec<(&str, &str)> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let end_of_sentence = matches!(bytes[i], b'.' | b'?' | b'!' | b':')
            && bytes.get(i + 1).is_some_and(|b| *b == b' ' || *b == b'\n');
        if end_of_sentence || bytes[i] == b'\n' {
            let piece_end = if bytes[i] == b'\n' { i } else { i + 1 };
            let mut gap_end = piece_end;
            while gap_end < bytes.len() && (bytes[gap_end] == b' ' || bytes[gap_end] == b'\n') {
                gap_end += 1;
            }
            pieces.push((&text[start..piece_end], &text[piece_end..gap_end]));
            start = gap_end;
            i = gap_end;
            continue;
        }
        i += 1;
    }
    if start < text.len() {
        pieces.push((&text[start..], ""));
    }
    pieces
}

/// Notes English text with no translation in the file AAE_L10N_MISSING
/// names, for translators.
fn note_missing(text: &str) {
    static PATH: LazyLock<Option<String>> =
        LazyLock::new(|| std::env::var("AAE_L10N_MISSING").ok().filter(|p| !p.is_empty()));
    let Some(path) = PATH.as_ref() else { return };
    if !text.chars().any(char::is_alphabetic) {
        return;
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", text.replace('\n', "\\n"));
    }
}

/// The msgid and msgstr of each entry of a gettext PO file, and the parts it
/// marks as messages of their own, in a "#. messages: why, 1" comment that
/// translations/extract.py writes from the code. An entry with a msgctxt has
/// "context\u{4}msgid" as its msgid, as gettext does.
fn po_entries(po: &str) -> Vec<(String, String, Vec<String>)> {
    let mut entries = Vec::new();
    let mut context: Option<String> = None;
    let mut id = String::new();
    let mut translated = String::new();
    // The messages comment before an entry, and the entry's own.
    let mut pending: Vec<String> = Vec::new();
    let mut messages: Vec<String> = Vec::new();
    // 0: msgctxt, 1: msgid, 2: msgstr.
    let mut field: Option<u8> = None;
    let flush = |entries: &mut Vec<(String, String, Vec<String>)>,
                 context: &mut Option<String>,
                 id: &mut String,
                 translated: &mut String,
                 messages: &mut Vec<String>| {
        if !id.is_empty() || !translated.is_empty() {
            let id = std::mem::take(id);
            let id = match context.take() {
                Some(c) => format!("{c}\u{4}{id}"),
                None => id,
            };
            entries.push((id, std::mem::take(translated), std::mem::take(messages)));
        }
        *context = None;
        messages.clear();
    };
    for line in po.lines().map(str::trim) {
        if line.is_empty() {
            flush(&mut entries, &mut context, &mut id, &mut translated, &mut messages);
            field = None;
            continue;
        }
        if line.starts_with('#') {
            if let Some(list) = line.strip_prefix("#. messages:") {
                pending = list
                    .split(',')
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("msgctxt ") {
            flush(&mut entries, &mut context, &mut id, &mut translated, &mut messages);
            messages = std::mem::take(&mut pending);
            context = Some(unquote(rest));
            field = Some(0);
        } else if let Some(rest) = line.strip_prefix("msgid ") {
            if field != Some(0) {
                flush(&mut entries, &mut context, &mut id, &mut translated, &mut messages);
                messages = std::mem::take(&mut pending);
            }
            id = unquote(rest);
            field = Some(1);
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            translated = unquote(rest);
            field = Some(2);
        } else if line.starts_with('"') {
            match field {
                Some(0) => {
                    if let Some(c) = context.as_mut() {
                        c.push_str(&unquote(line));
                    }
                }
                Some(1) => id.push_str(&unquote(line)),
                Some(2) => translated.push_str(&unquote(line)),
                _ => {}
            }
        }
    }
    flush(&mut entries, &mut context, &mut id, &mut translated, &mut messages);
    entries
}

fn unquote(text: &str) -> String {
    let inner = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text);
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(po: &str) -> Catalog {
        Catalog::parse(po)
    }

    const PO: &str = r#"
msgid ""
msgstr "Content-Type: text/plain; charset=UTF-8\n"

msgid "Cold Boot"
msgstr "Démarrage à froid"

msgid "{device} is ready. {reader} is on."
msgstr "L'appareil {device} est prêt. {reader} est activé."

#. messages: why
msgid "Couldn't start {device}: {why}"
msgstr "Impossible de démarrer l'appareil {device} : {why}"

msgid "The emulator isn't running."
msgstr "L'émulateur ne tourne pas."

msgid "%@ of %@"
msgstr "%2$@ sur %1$@"

msgid "Phone"
msgstr "Téléphone"

msgid "16 KB pages"
msgstr "pages de 16 Ko"

msgid " Using NVDA."
msgstr " NVDA est utilisé."

msgid "Installing AAE's helper"
msgstr "Installation de l'app auxiliaire d'AAE"

msgid "Warning"
msgstr "Avertissement"

msgid "Warning: {} is old."
msgstr "Avertissement : {} est ancien."

msgid "Opened {name}."
msgstr "{name} est ouvert."

msgid "{} plays audio at the right speed."
msgstr "{} lit le son à la bonne vitesse."

msgid "full speed"
msgstr "pleine vitesse"

msgid "{} processor threads{memory}. Devices get {} cores."
msgstr "{} processeurs logiques{memory}. Les appareils utilisent {} cœurs."

msgid "It is running{}."
msgstr "L'appareil est en marche{}."

msgid "{} installed."
msgstr "Installées : {}."

#. messages: 0
msgid "Problem: {}"
msgstr "Problème : {}"

msgid "Not found."
msgstr "Introuvable."

msgid "Android versions"
msgstr "Versions d'Android"

msgid "Android has started."
msgstr "Android a démarré."

msgid "Watch"
msgstr "Montre"

msgctxt "start watching for builds"
msgid "Watch"
msgstr "Surveiller"

msgid "Line one\n"
"line two"
msgstr "Ligne un\n"
"ligne deux"
"#;

    #[test]
    fn exact_text() {
        let c = catalog(PO);
        assert_eq!(c.translate("Cold Boot"), "Démarrage à froid");
        assert_eq!(c.translate("Line one\nline two"), "Ligne un\nligne deux");
        assert_eq!(c.translate("Not in the catalog"), "Not in the catalog");
    }

    #[test]
    fn named_parts_go_in_their_place() {
        let c = catalog(PO);
        assert_eq!(
            c.translate("Pixel 9 is ready. TalkBack is on."),
            "L'appareil Pixel 9 est prêt. TalkBack est activé."
        );
    }

    #[test]
    fn parts_that_are_messages_are_translated_too() {
        let c = catalog(PO);
        assert_eq!(
            c.translate("Couldn't start Test: The emulator isn't running."),
            "Impossible de démarrer l'appareil Test : L'émulateur ne tourne pas."
        );
    }

    #[test]
    fn swift_positions_can_change_order() {
        let c = catalog(PO);
        assert_eq!(c.translate("3 of 5"), "5 sur 3");
    }

    #[test]
    fn joined_messages_are_translated_one_by_one() {
        let c = catalog(PO);
        assert_eq!(
            c.translate("Android has started. Cold Boot"),
            "Android a démarré. Démarrage à froid"
        );
    }

    #[test]
    fn lists_are_translated_item_by_item() {
        let c = catalog(PO);
        assert_eq!(
            c.translate("16 KB pages, Phone. Android has started."),
            "pages de 16 Ko, Téléphone. Android a démarré."
        );
        assert_eq!(
            c.translate("Android 17 (API 37.2), 16 KB pages, Phone"),
            "Android 17 (API 37.2), pages de 16 Ko, Téléphone"
        );
    }

    #[test]
    fn spaces_and_final_periods_dont_hide_a_text() {
        let c = catalog(PO);
        assert_eq!(c.translate("Using NVDA."), "NVDA est utilisé.");
        assert_eq!(
            c.translate("Installing AAE's helper."),
            "Installation de l'app auxiliaire d'AAE."
        );
        assert_eq!(c.translate("Android has started"), "Android a démarré");
    }

    #[test]
    fn labels_and_runs_of_sentences() {
        let c = catalog(PO);
        assert_eq!(
            c.translate("Warning: Android has started. 18 processor threads, 64 GB. Devices get 4 cores."),
            "Avertissement\u{a0}: Android a démarré. 18 processeurs logiques, 64 GB. Les appareils utilisent 4 cœurs."
        );
    }

    #[test]
    fn joining_names_never_takes_sentences() {
        let c = catalog(
            "msgid \"{} and {}\"\nmsgstr \"{} et {}\"\n\nmsgid \"Android has started.\"\nmsgstr \"Android a démarré.\"\n",
        );
        assert_eq!(c.translate("Test and Pixel"), "Test et Pixel");
        assert_eq!(
            c.translate("Up and away. Android has started."),
            "Up et away. Android a démarré."
        );
    }

    #[test]
    fn names_stay_as_they_are() {
        let c = catalog(PO);
        // A part not marked as a message is a name, even one AAE knows.
        assert_eq!(c.translate("Opened Watch."), "Watch est ouvert.");
        assert_eq!(c.translate("Phone plays audio at the right speed."), "Phone lit le son à la bonne vitesse.");
        // So is a device's name before a colon: only known labels change.
        assert_eq!(
            c.translate("Phone: Android 17 (API 37.2), 16 KB pages, Phone"),
            "Phone: Android 17 (API 37.2), pages de 16 Ko, Téléphone"
        );
        // A part marked as a message is translated.
        assert_eq!(
            c.translate("Couldn't start Watch: The emulator isn't running."),
            "Impossible de démarrer l'appareil Watch : L'émulateur ne tourne pas."
        );
    }

    #[test]
    fn tr_format_keeps_its_values() {
        let c = catalog(PO);
        // A format the catalog has, with a name that is also a word it knows.
        assert_eq!(tr_format_from(Some(&c), "Opened {name}.", &["Watch"]), "Watch est ouvert.");
        assert_eq!(tr_format_from(Some(&c), "%@ of %@", &["3", "5"]), "5 sur 3");
        assert_eq!(tr_format_from(Some(&c), "  Opened {name}.", &["Pixel"]), "  Pixel est ouvert.");
        // One it doesn't have, as it is.
        assert_eq!(tr_format_from(Some(&c), "{} ({})", &["Phone", "com.example"]), "Phone (com.example)");
        // In English.
        assert_eq!(tr_format_from(None, "Opened {name}.", &["Watch"]), "Opened Watch.");
    }

    #[test]
    fn messages_comment_marks_its_own_entry() {
        let entries = po_entries(
            "#. messages: why, 1\nmsgid \"A {why} {}\"\nmsgstr \"B\"\n\nmsgid \"C {}\"\nmsgstr \"D\"\n",
        );
        assert_eq!(entries[0].2, vec!["why".to_string(), "1".to_string()]);
        assert!(entries[1].2.is_empty());
    }

    #[test]
    fn a_large_dump_is_looked_up_quickly() {
        // An inspector dump: thousands of lines, none of them AAE's.
        let fr = Catalog::parse(CATALOGS[0].1);
        let dump: String = (0..5000)
            .map(|i| format!("  Button {i}, \"Item {i}\", clickable, at 10, {i}. Settings\n"))
            .collect();
        let started = std::time::Instant::now();
        assert_eq!(fr.translate(&dump), dump);
        // Under the limits, a long text is still looked up line by line, and
        // its cost grows with its length.
        let text: String = (0..150).map(|i| format!("Line {i} of nothing known.\n")).collect();
        assert_eq!(fr.translate(&text), text);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[test]
    fn a_value_shown_with_a_capital() {
        let c = catalog(PO);
        assert_eq!(c.translate("Full speed"), "Pleine vitesse");
    }

    #[test]
    fn a_part_after_fixed_text_may_be_empty() {
        let c = catalog(PO);
        assert_eq!(c.translate("It is running."), "L'appareil est en marche.");
    }

    #[test]
    fn a_leading_part_never_takes_an_earlier_sentence() {
        let c = catalog(PO);
        assert_eq!(
            c.translate("Android versions. 1 installed."),
            "Versions d'Android. Installées : 1."
        );
        assert_eq!(
            c.translate("Problem: Android has started. Not found."),
            "Problème : Android a démarré. Introuvable."
        );
    }

    #[test]
    fn spaces_around_are_kept() {
        let c = catalog(PO);
        assert_eq!(c.translate("  Cold Boot\n"), "  Démarrage à froid\n");
    }

    #[test]
    fn context_tells_meanings_apart() {
        let c = catalog(PO);
        assert_eq!(c.translate("Watch"), "Montre");
        assert_eq!(
            c.exact.get("start watching for builds\u{4}Watch").map(String::as_str),
            Some("Surveiller")
        );
    }

    #[test]
    fn language_tags() {
        assert!(set_language("fr_FR.UTF-8"));
        assert_eq!(language(), Some("fr"));
        assert!(!set_language("de-DE"));
        assert_eq!(language(), None);
        assert_eq!(tr("Cold Boot"), "Cold Boot");
    }

    /// For translators: each line of AAE_L10N_TRY in French, with
    /// `cargo test -p aae-core l10n::tests::try_text -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn try_text() {
        let fr = Catalog::parse(CATALOGS[0].1);
        for line in std::env::var("AAE_L10N_TRY").unwrap_or_default().lines() {
            println!("{line}\n  → {}", fr.translate(line));
        }
    }

    /// Every translation keeps the English text's filled-in parts, no more
    /// and no fewer.
    #[test]
    fn catalogs_keep_every_part() {
        let slots = |text: &str| {
            let mut names: Vec<String> = parts(text)
                .into_iter()
                .filter_map(|p| match p {
                    Part::Slot(name) => Some(name),
                    Part::Text(_) => None,
                })
                .collect();
            names.sort();
            names
        };
        for (code, po) in CATALOGS {
            for (english, translated, _) in po_entries(po) {
                if english.is_empty() || translated.is_empty() {
                    continue;
                }
                assert_eq!(
                    slots(&english),
                    slots(&translated),
                    "{code}: {english:?} and {translated:?} have different parts"
                );
            }
        }
    }
}
