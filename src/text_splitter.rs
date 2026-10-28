const SENTENCE_ENDINGS: [char; 7] = ['。', '！', '？', '.', '!', '?', '\n'];
const CLOSING_CHARS: [char; 8] = ['」', '』', '）', ')', '】', '"', '\'', '”'];

/// Splits text into sentences so that each one can be synthesized and played independently.
/// Sentences are never split in the middle, so that intonation is predicted from the whole
/// sentence (long sentences are segmented by `Engine::synthesize` after analysis instead).
/// Chunks without any pronounceable characters (e.g. a lone "」") are dropped.
pub fn split_text(text: &str) -> Vec<String> {
    split_into_sentences(text)
        .into_iter()
        .map(|chunk| chunk.trim().to_string())
        .filter(|chunk| chunk.chars().any(char::is_alphanumeric))
        .collect()
}

fn split_into_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current_sentence = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        current_sentence.push(ch);

        if SENTENCE_ENDINGS.contains(&ch) {
            // Keep closing brackets/quotes with the sentence they close.
            while let Some(&next) = chars.peek() {
                if CLOSING_CHARS.contains(&next) || SENTENCE_ENDINGS.contains(&next) {
                    current_sentence.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            sentences.push(std::mem::take(&mut current_sentence));
        }
    }

    if !current_sentence.trim().is_empty() {
        sentences.push(current_sentence);
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_by_sentence() {
        assert_eq!(
            split_text("こんにちは。元気ですか？はい！"),
            ["こんにちは。", "元気ですか？", "はい！"]
        );
    }

    #[test]
    fn keeps_closing_brackets_with_sentence() {
        assert_eq!(
            split_text("彼は「行くよ。」と言った。"),
            ["彼は「行くよ。」", "と言った。"]
        );
    }

    #[test]
    fn splits_on_newlines_and_drops_empty_chunks() {
        assert_eq!(split_text("一行目\n\n二行目\n"), ["一行目", "二行目"]);
        assert!(split_text("。。」\n").is_empty());
    }

    #[test]
    fn text_without_sentence_ending_is_one_chunk() {
        assert_eq!(split_text("こんにちは"), ["こんにちは"]);
    }

    #[test]
    fn long_sentence_is_not_split() {
        let part = "あ".repeat(60);
        let text = format!("{part}、{part}、{part}。");
        assert_eq!(split_text(&text), [text]);
    }
}
