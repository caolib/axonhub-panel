use super::*;

fn runs(s: &str) -> Vec<(&str, bool)> {
    split_runs(s)
}

#[test]
fn ascii_only_is_one_run() {
    assert_eq!(runs("claude-opus-5"), vec![("claude-opus-5", false)]);
    assert_eq!(runs("3.4s"), vec![("3.4s", false)]);
}

#[test]
fn chinese_only_is_one_run() {
    assert_eq!(runs("已完成"), vec![("已完成", true)]);
}

#[test]
fn mixed_text_alternates_runs() {
    // The label shape the panel actually draws.
    assert_eq!(runs("缓存 99.5%"), vec![("缓存", true), (" 99.5%", false)]);
    // The space after a digit stays with the Latin run; it is only the
    // run boundaries that matter for the face, not which side the space
    // lands on.
    assert_eq!(runs("5 分钟前"), vec![("5 ", false), ("分钟前", true)]);
}

#[test]
fn preserved_text_is_byte_exact() {
    // Concatenating the runs must reproduce the input, or text would be
    // silently mangled when drawn piecewise.
    for s in ["缓存 99.5%", "5 分钟前", "cc · alice", "透传", "abc", ""] {
        let joined: String = runs(s).into_iter().map(|(t, _)| t).collect();
        assert_eq!(joined, s, "round trip failed for {s:?}");
    }
}

#[test]
fn full_width_punctuation_counts_as_cjk() {
    // `·` is U+00B7 (Latin), but `：` is full-width and needs the CJK face.
    assert_eq!(runs("凭据:token"), vec![("凭据", true), (":token", false)]);
    assert_eq!(runs("凭据："), vec![("凭据：", true)]);
}

#[test]
fn leading_and_trailing_spaces_stay_with_their_run() {
    assert_eq!(
        runs(" 完成 "),
        vec![(" ", false), ("完成", true), (" ", false)]
    );
}

/// Stand-in for the monospaced face: one unit per character, so a test
/// budget of N * 10 is exactly N characters.
fn mono(s: &str) -> f32 {
    s.chars().count() as f32 * 10.0
}

#[test]
fn wraps_latin_between_words() {
    assert_eq!(
        wrap_by("alpha beta gamma", 100.0, mono),
        vec!["alpha beta", "gamma"]
    );
}

#[test]
fn wraps_cjk_between_characters() {
    assert_eq!(wrap_by("中文中文", 20.0, mono), vec!["中文", "中文"]);
}

#[test]
fn breaks_a_token_wider_than_the_column() {
    let url = "https://example.com/very/long/path";
    let lines = wrap_by(url, 40.0, mono);
    assert!(lines.len() > 1);
    assert!(lines.iter().all(|l| mono(l) <= 40.0), "{lines:?}");
    assert_eq!(lines.concat(), url);
}

#[test]
fn newlines_start_a_new_line() {
    assert_eq!(wrap_by("a\nb", 100.0, mono), vec!["a", "b"]);
    // An empty paragraph survives as an empty line.
    assert_eq!(wrap_by("a\n\nb", 100.0, mono), vec!["a", "", "b"]);
}

#[test]
fn wrapping_keeps_every_character() {
    for text in [
        "failed to do request: HTTP request failed: Post \"https://api.example.com/v1/chat/completions?beta=true\": context canceled",
        "Concurrency limit exceeded for account, please retry later",
        "模型 glm-5.3-flash 在渠道 aliyun 上返回 429",
    ] {
        let joined: String = wrap_by(text, 120.0, mono).join(" ");
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(strip(&joined), strip(text), "text was mangled: {text}");
    }
}
