// Finding the wake phrase in a transcript, by the rules of VOICE §6. Package voice-wake. Ported from
// upstream's `Voice/WakePhrase.swift`: the scan at a word boundary anywhere in the transcript, the last
// occurrence winning, the command taken from the original text after it. The word lists are the
// spike's (the voice research, §13.4). Pure text, no audio: its probe is a table of sentences.
//
// The phrase is "OK " + the first word of `notch_brand::NAME` (a name of several words is said by its
// first: "OK METEOR"); the phrase, the keywords and the distance rule follow it, the table of
// mishearings is per name.

/// A wake found in a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wake {
    /// Byte offset of the phrase in the transcript (the last occurrence wins).
    pub at: usize,
    /// The text after it: the command, trimmed, maybe empty.
    pub rest: String,
}

/// What the engine writes for the first word.
const OK: &[&str] = &["ok", "okay", "oaky", "okie", "o", "oke", "okey"];

/// What the engine may write for the name. Per name, and measured with target/voice-spike/scripts/.
// debt: not measured for this name yet (the code name's table came from 63 clips); these are guesses.
// The distance rule below covers the one-letter slips.
const MISHEARD: &[&str] = &["meteo", "metior", "meatier"];

/// A stock phrase the engine writes on hum and clicks.
const STOCK: &[&str] = &["thank you", "thanks for watching", "you", "bye"];

/// The name as it is said: its first word.
fn spoken() -> &'static str {
    notch_brand::NAME.split_whitespace().next().unwrap_or(notch_brand::NAME)
}

/// "OK METEOR": what the Settings page shows and the first keyword.
pub fn phrase() -> String {
    format!("OK {}", spoken())
}

/// For `SttOptions::keywords`: the phrase, the noun.
pub fn keywords() -> Vec<String> {
    vec![phrase(), spoken().to_string()]
}

/// The words of a transcript, lowercase and without punctuation, each with where it starts and ends.
fn words(text: &str) -> Vec<(usize, usize, String)> {
    let mut words: Vec<(usize, usize, String)> = Vec::new();
    let mut open = false;
    for (at, c) in text.char_indices() {
        if !c.is_alphanumeric() {
            open = false;
            continue;
        }
        if !open {
            words.push((at, at, String::new()));
        }
        open = true;
        if let Some(word) = words.last_mut() {
            word.1 = at + c.len_utf8();
            word.2.extend(c.to_lowercase());
        }
    }
    words
}

/// At most one letter apart: one changed, one missing or one extra.
fn near(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let same = short.iter().zip(long.iter()).take_while(|(x, y)| x == y).count();
    // Past the first difference the rest agrees: letter for letter, or with the longer one a letter ahead.
    let changed = (long.len() == short.len()) as usize;
    short[(same + changed).min(short.len())..] == long[(same + 1).min(long.len())..]
}

/// The word is the name, as the engine may have written it.
// debt: a name of several words would need its words matched one by one.
fn named(word: &str) -> bool {
    let name = spoken().to_lowercase();
    near(word, &name) || MISHEARD.contains(&word)
}

/// The phrase anywhere at a word boundary, by the two word lists and the distance rule.
pub fn find(transcript: &str) -> Option<Wake> {
    let words = words(transcript);
    let hit = words.windows(2).rev().find(|pair| OK.contains(&pair[0].2.as_str()) && named(&pair[1].2))?;
    let rest = transcript[hit[1].1..].trim_start_matches(|c: char| !c.is_alphanumeric()).trim_end();
    Some(Wake { at: hit[0].0, rest: rest.to_string() })
}

/// An OK word is somewhere in the transcript: what the second pass, without keywords, must still hear.
pub fn ok_word(transcript: &str) -> bool {
    words(transcript).iter().any(|word| OK.contains(&word.2.as_str()))
}

/// The transcript is only a stock phrase the engine writes on hum ("thank you", "bye"…): dropped when it
/// comes from under 0.6 s of voice.
pub fn stock(transcript: &str) -> bool {
    let said: Vec<String> = words(transcript).into_iter().map(|word| word.2).collect();
    STOCK.contains(&said.join(" ").as_str())
}
