// SPDX-License-Identifier: MPL-2.0
//! Remove recognizer-inserted CJK word separators, not user layout or Latin spaces.
//! Keep literal words and their boxes unchanged so selection offsets stay recoverable.
use mewu_core::{OcrDocument, OcrLine, OcrWord};

pub(crate) fn normalize(document: &mut OcrDocument) {
    for line in &mut document.lines {
        normalize_line(line);
    }
}

fn cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
        | 0x20000..=0x323af | 0x3041..=0x3096 | 0x30a1..=0x30fa)
}

fn joined(left: char, right: char) -> bool {
    (cjk(left) && cjk(right))
        || (cjk(left) && "，。！？；：、）》】」』…".contains(right))
        || ("（《【「『".contains(left) && cjk(right))
        || ("，。！？；：、）》】」』…".contains(left) && cjk(right))
}

fn adjacent(left: &OcrWord, right: &OcrWord) -> bool {
    let values = [left.x, left.y, left.width, left.height, right.x, right.y, right.width, right.height];
    if values.iter().any(|value| !value.is_finite())
        || left.width <= 0. || right.width <= 0. || left.height <= 0. || right.height <= 0. {
        return false;
    }
    let height = left.height.min(right.height);
    let overlap = (left.y + left.height).min(right.y + right.height) - left.y.max(right.y);
    let gap = right.x - (left.x + left.width);
    let advance = (left.width / left.text.chars().count().max(1) as f64)
        .min(right.width / right.text.chars().count().max(1) as f64);
    // Fullwidth punctuation may have only a tiny ink box at the baseline.
    // Its ink width is not its typographic advance. Use the adjacent CJK
    // character's em for that pair, retaining the same line/column checks.
    let left_cjk = left.text.chars().next_back().is_some_and(cjk);
    let right_cjk = right.text.chars().next().is_some_and(cjk);
    let gap_limit = if left_cjk != right_cjk {
        let text = if left_cjk { left } else { right };
        (text.width / text.text.chars().count().max(1) as f64).min(text.height)
    } else { advance.min(height) * 0.5 };
    overlap >= height * 0.5 && right.x >= left.x && gap >= -height * 0.25
        && gap <= gap_limit
}

fn normalize_line(line: &mut OcrLine) {
    let mut previous: Option<(&OcrWord, usize)> = None;
    let mut offset = 0;
    let mut removals = Vec::new();
    for word in &line.words {
        if word.text.is_empty() { previous = None; continue; }
        let Some(found) = line.text[offset..].find(&word.text) else {
            // Do not guess offsets when the recognizer's words disagree with its line.
            return;
        };
        let start = offset + found;
        if let Some((left, end)) = previous {
            if &line.text[end..start] == " "
                && joined(left.text.chars().next_back().unwrap(), word.text.chars().next().unwrap())
                && adjacent(left, word) {
                removals.push(end);
            }
        }
        offset = start + word.text.len();
        previous = Some((word, offset));
    }
    if removals.is_empty() { return; }
    let mut output = String::with_capacity(line.text.len() - removals.len());
    let mut start = 0;
    for end in removals {
        output.push_str(&line.text[start..end]);
        start = end + 1; // Only a verified ASCII space is removed.
    }
    output.push_str(&line.text[start..]);
    line.text = output;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(text: &str, words: &[&str]) -> OcrLine {
        let mut x = 0.;
        OcrLine { text: text.into(), words: words.iter().map(|text| {
            let width = text.chars().count() as f64 * 20.;
            let word = OcrWord { text: (*text).into(), x, y: 5., width, height: 20. };
            x += width + 2.; word
        }).collect() }
    }
    #[test]
    fn removes_only_close_cjk_separators_without_changing_boxes_or_words() {
        let mut value = line("你 好 世界 ， 测 试", &["你", "好", "世界", "，", "测", "试"]);
        let original = serde_json::to_value(&value.words).unwrap();
        normalize_line(&mut value);
        assert_eq!(value.text, "你好世界，测试");
        assert_eq!(serde_json::to_value(&value.words).unwrap(), original);
        normalize_line(&mut value);
        assert_eq!(value.text, "你好世界，测试");
    }
    #[test]
    fn preserves_latin_korean_numbers_indentation_tabs_and_column_spacing() {
        for (text, words) in [
            ("Hello world", vec!["Hello", "world"]),
            ("안녕 세계", vec!["안녕", "세계"]),
            ("版本 Windows 11", vec!["版本", "Windows", "11"]),
            ("  姓名  成绩\t备注", vec!["姓名", "成绩", "备注"]),
            ("1 234", vec!["1", "234"]),
        ] {
            let mut value = line(text, &words); normalize_line(&mut value); assert_eq!(value.text, text);
        }
    }
    #[test]
    fn wide_columns_vertical_text_and_invalid_boxes_are_not_joined() {
        for mode in 0..3 {
            let mut value = line("姓名 成绩", &["姓名", "成绩"]);
            match mode { 0 => value.words[1].x += 100., 1 => value.words[1].y += 40., _ => value.words[1].width = f64::NAN }
            normalize_line(&mut value); assert_eq!(value.text, "姓名 成绩");
        }
    }
    #[test]
    fn repeated_words_supplementary_han_and_japanese_keep_utf8_offsets() {
        for (text, words, expected) in [
            ("好 好 学习", vec!["好", "好", "学习"], "好好学习"),
            ("𠮷 野 家", vec!["𠮷", "野", "家"], "𠮷野家"),
            ("「 日 本 語 」", vec!["「", "日", "本", "語", "」"], "「日本語」"),
        ] { let mut value = line(text, &words); normalize_line(&mut value); assert_eq!(value.text, expected); }
    }
    #[test]
    fn inconsistent_words_preserve_the_complete_original_line() {
        let mut value = line("你 好 世界", &["你", "好", "错误"]);
        normalize_line(&mut value); assert_eq!(value.text, "你 好 世界");
    }

    #[test]
    fn narrow_baseline_punctuation_uses_the_neighboring_cjk_advance() {
        let mut value = OcrLine { text: "界 ， 文".into(), words: vec![
            OcrWord { text: "界".into(), x: 118., y: 35., width: 29., height: 27. },
            OcrWord { text: "，".into(), x: 151., y: 54., width: 5., height: 8. },
            OcrWord { text: "文".into(), x: 177., y: 31., width: 29., height: 29. },
        ] };
        let mut columns = value.clone();
        columns.words[2].x += 100.;
        normalize_line(&mut value);
        assert_eq!(value.text, "界，文");
        normalize_line(&mut columns);
        assert_eq!(columns.text, "界， 文");
    }
}
