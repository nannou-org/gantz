//! Syntax colours for scheme text. [`tokens`] splits text into [`Token`]s and
//! [`job`] sets them in the colours of the style.
//!
//! The lexer keeps no state but a [`Mode`]. A view of a long text can
//! therefore colour any one line on its own, given the mode at its start.

use crate::style::{self, role::Role};
use egui::text::{LayoutJob, LayoutSection, TextFormat};
use std::ops::Range;

/// The class of a run of scheme text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Space,
    Comment,
    String,
    /// A number, boolean, character or keyword argument.
    Literal,
    /// The name of a special form, for example `define`.
    Keyword,
    /// A bracket or a quote mark.
    Bracket,
    Symbol,
}

/// What the lexer is inside of at a point in the text. Strings and block
/// comments can span lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Code,
    String,
    /// A `#|` block comment, with the count of comments open around the
    /// point.
    Comment(u32),
}

/// The font and colours of scheme text.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub font: egui::FontId,
    pub symbol: egui::Color32,
    pub comment: egui::Color32,
    pub string: egui::Color32,
    pub literal: egui::Color32,
    pub keyword: egui::Color32,
    pub bracket: egui::Color32,
}

impl Theme {
    /// The monospace font and the syntax colours of the style of `ui`.
    pub fn of(ui: &egui::Ui) -> Self {
        let [symbol, comment, string, literal, keyword, bracket] = style::colors(
            ui,
            [
                Role::Text,
                Role::SyntaxComment,
                Role::SyntaxString,
                Role::SyntaxLiteral,
                Role::SyntaxKeyword,
                Role::SyntaxBracket,
            ],
        );
        Self {
            font: egui::TextStyle::Monospace.resolve(ui.style()),
            symbol,
            comment,
            string,
            literal,
            keyword,
            bracket,
        }
    }

    /// The colour of `token`. Space takes the colour of symbols, for it shows
    /// only where a background is set.
    pub fn color(&self, token: Token) -> egui::Color32 {
        match token {
            Token::Space | Token::Symbol => self.symbol,
            Token::Comment => self.comment,
            Token::String => self.string,
            Token::Literal => self.literal,
            Token::Keyword => self.keyword,
            Token::Bracket => self.bracket,
        }
    }
}

/// The tokens of `src` in order, each with its byte range and the mode after
/// it. The lexer starts in `mode`. The ranges cover `src` with no gaps.
pub fn tokens(src: &str, mode: Mode) -> impl Iterator<Item = (Token, Range<usize>, Mode)> + '_ {
    let mut state = (0, mode);
    std::iter::from_fn(move || {
        let (start, mode) = state;
        let rest = src.get(start..).filter(|rest| !rest.is_empty())?;
        let (token, len, mode) = step(rest, mode);
        state = (start + len, mode);
        Some((token, start..start + len, mode))
    })
}

/// The mode at the end of `src`, for a lexer that starts in `mode`.
pub fn mode_after(src: &str, mode: Mode) -> Mode {
    tokens(src, mode).last().map_or(mode, |(_, _, mode)| mode)
}

/// `src` set in the font and colours of `theme`, for a lexer that starts in
/// `mode`.
pub fn job(src: &str, mode: Mode, theme: &Theme) -> LayoutJob {
    let mut sections: Vec<LayoutSection> = vec![];
    for (token, range, _) in tokens(src, mode) {
        let color = theme.color(token);
        match sections.last_mut() {
            // Space shows no colour and joins the section before it.
            Some(last) if token == Token::Space || last.format.color == color => {
                last.byte_range.end = range.end;
            }
            _ => sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: range,
                format: TextFormat::simple(theme.font.clone(), color),
            }),
        }
    }
    LayoutJob {
        text: src.to_string(),
        sections,
        ..Default::default()
    }
}

/// The token at the start of `src`, its length in bytes and the mode after
/// it. `src` is not empty.
fn step(src: &str, mode: Mode) -> (Token, usize, Mode) {
    let bytes = src.as_bytes();
    match mode {
        Mode::String => string(bytes, 0),
        Mode::Comment(depth) => block_comment(bytes, 0, depth),
        Mode::Code => match bytes {
            [b, ..] if b.is_ascii_whitespace() => {
                let len = run(bytes, |b| b.is_ascii_whitespace());
                (Token::Space, len, Mode::Code)
            }
            [b';', ..] => (Token::Comment, run(bytes, |b| b != b'\n'), Mode::Code),
            [b'"', ..] => string(bytes, 1),
            [b'#', b'|', ..] => block_comment(bytes, 2, 1),
            [b'#', b';', ..] => (Token::Comment, 2, Mode::Code),
            [b'#', b'\\', ..] => {
                // The character after the backslash may itself be a delimiter.
                let first = src[2..].chars().next().map_or(0, char::len_utf8);
                let len = 2 + first + run(&bytes[2 + first..], |b| !is_delimiter(b));
                (Token::Literal, len, Mode::Code)
            }
            [b',', b'@', ..] => (Token::Bracket, 2, Mode::Code),
            [
                b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'\'' | b'`' | b',',
                ..,
            ] => (Token::Bracket, 1, Mode::Code),
            _ => {
                let len = run(bytes, |b| !is_delimiter(b));
                (atom(&src[..len]), len, Mode::Code)
            }
        },
    }
}

/// A string token of `bytes` whose content starts at `from`, up to its
/// closing quote or the end of `bytes`.
fn string(bytes: &[u8], from: usize) -> (Token, usize, Mode) {
    let mut i = from;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'"' => return (Token::String, i + 1, Mode::Code),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    (Token::String, bytes.len(), Mode::String)
}

/// A block comment token of `bytes` from `from` on, with `depth` comments
/// open there. It runs up to the close of the outermost one or the end of
/// `bytes`.
fn block_comment(bytes: &[u8], from: usize, depth: u32) -> (Token, usize, Mode) {
    let (mut i, mut depth) = (from, depth);
    while let Some(pair) = bytes.get(i..i + 2) {
        match pair {
            b"|#" if depth == 1 => return (Token::Comment, i + 2, Mode::Code),
            b"|#" => (i, depth) = (i + 2, depth - 1),
            b"#|" => (i, depth) = (i + 2, depth + 1),
            _ => i += 1,
        }
    }
    (Token::Comment, bytes.len(), Mode::Comment(depth))
}

/// The class of a token that is no space, comment, string or bracket.
fn atom(text: &str) -> Token {
    match text.as_bytes() {
        [b'#', ..] => Token::Literal,
        [d, ..] | [b'+' | b'-' | b'.', d, ..] | [b'+' | b'-', b'.', d, ..]
            if d.is_ascii_digit() =>
        {
            Token::Literal
        }
        _ if is_keyword(text) => Token::Keyword,
        _ => Token::Symbol,
    }
}

/// Whether `text` names a special form of steel or one of its common macros.
fn is_keyword(text: &str) -> bool {
    matches!(
        text,
        "and"
            | "begin"
            | "case"
            | "cond"
            | "define"
            | "define-syntax"
            | "define-values"
            | "do"
            | "else"
            | "fn"
            | "if"
            | "lambda"
            | "let"
            | "let*"
            | "let-values"
            | "letrec"
            | "letrec*"
            | "or"
            | "provide"
            | "quasiquote"
            | "quote"
            | "require"
            | "return!"
            | "set!"
            | "syntax-rules"
            | "unless"
            | "unquote"
            | "when"
            | "while"
            | "λ"
    )
}

/// Whether `b` ends the token before it. Every delimiter is ASCII. A token
/// that ends at one therefore ends on a character boundary.
fn is_delimiter(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'"' | b';')
}

/// The length of the longest prefix of `bytes` whose bytes all pass `keep`.
fn run(bytes: &[u8], keep: impl Fn(u8) -> bool) -> usize {
    bytes.iter().take_while(|&&b| keep(b)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Token::*;

    fn lex(src: &str) -> Vec<(Token, &str)> {
        tokens(src, Mode::Code)
            .filter(|(token, ..)| *token != Space)
            .map(|(token, range, _)| (token, &src[range]))
            .collect()
    }

    #[test]
    fn classes_of_a_definition() {
        assert_eq!(
            lex("(define node-fn-6:1 (λ (input0) (* 440.0 'a \"s\")))"),
            vec![
                (Bracket, "("),
                (Keyword, "define"),
                (Symbol, "node-fn-6:1"),
                (Bracket, "("),
                (Keyword, "λ"),
                (Bracket, "("),
                (Symbol, "input0"),
                (Bracket, ")"),
                (Bracket, "("),
                (Symbol, "*"),
                (Literal, "440.0"),
                (Bracket, "'"),
                (Symbol, "a"),
                (String, "\"s\""),
                (Bracket, ")"),
                (Bracket, ")"),
                (Bracket, ")"),
            ]
        );
    }

    #[test]
    fn literals() {
        for src in [
            "1", "-1", "+.5", "-0.5", "1/2", "1e3", "#t", "#false", "#x1F", "#:init", "#\\a",
            "#\\(", "#\\space", "#\\λ",
        ] {
            assert_eq!(lex(src), vec![(Literal, src)], "{src}");
        }
        for src in ["-", "+", "...", "$x", "a1", "-a"] {
            assert_eq!(lex(src), vec![(Symbol, src)], "{src}");
        }
    }

    #[test]
    fn comments() {
        assert_eq!(
            lex("a ; b (c\nd #;(e) #| f #| g |# h |# i"),
            vec![
                (Symbol, "a"),
                (Comment, "; b (c"),
                (Symbol, "d"),
                (Comment, "#;"),
                (Bracket, "("),
                (Symbol, "e"),
                (Bracket, ")"),
                (Comment, "#| f #| g |# h |#"),
                (Symbol, "i"),
            ]
        );
    }

    #[test]
    fn string_escapes() {
        assert_eq!(
            lex(r#"("a\"b" "\\" c)"#),
            vec![
                (Bracket, "("),
                (String, r#""a\"b""#),
                (String, r#""\\""#),
                (Symbol, "c"),
                (Bracket, ")"),
            ]
        );
    }

    /// Lexing a text line by line, with each line starting in the mode the
    /// line before left, gives the classes of lexing it whole.
    #[test]
    fn modes_carry_strings_and_comments_across_lines() {
        let src = "(a \"b\nc\" d #| e\n#| f\n|# g |# h)\n\"i\\";
        let whole: Vec<(Token, &str)> = tokens(src, Mode::Code)
            .flat_map(|(token, range, _)| src[range].split('\n').map(move |part| (token, part)))
            .filter(|(token, part)| *token != Space && !part.is_empty())
            .collect();
        let mut mode = Mode::Code;
        let mut by_line = vec![];
        let mut modes = vec![];
        for line in src.split_inclusive('\n') {
            modes.push(mode);
            for (token, range, after) in tokens(line, mode) {
                let part = line[range].trim_end_matches('\n');
                if token != Space && !part.is_empty() {
                    by_line.push((token, part));
                }
                mode = after;
            }
        }
        assert_eq!(by_line, whole);
        assert_eq!(
            modes,
            vec![
                Mode::Code,
                Mode::String,
                Mode::Comment(1),
                Mode::Comment(2),
                Mode::Code
            ]
        );
        assert_eq!(mode_after(src, Mode::Code), Mode::String);
    }

    #[test]
    fn ranges_cover_the_text() {
        let src = "(λ (x) \"é\\\n #\\é ; ü\n #| ö";
        let mut end = 0;
        for (_, range, _) in tokens(src, Mode::Code) {
            assert_eq!(range.start, end);
            assert!(src.is_char_boundary(range.end));
            end = range.end;
        }
        assert_eq!(end, src.len());
    }

    #[test]
    fn job_sections_cover_the_text() {
        let theme = Theme {
            font: egui::FontId::monospace(12.0),
            symbol: egui::Color32::from_gray(1),
            comment: egui::Color32::from_gray(2),
            string: egui::Color32::from_gray(3),
            literal: egui::Color32::from_gray(4),
            keyword: egui::Color32::from_gray(5),
            bracket: egui::Color32::from_gray(6),
        };
        let src = "  (define x 1) ; done";
        let job = job(src, Mode::Code, &theme);
        let sections: Vec<_> = job
            .sections
            .iter()
            .map(|s| (&src[s.byte_range.clone()], s.format.color))
            .collect();
        assert_eq!(
            sections,
            vec![
                ("  ", theme.symbol),
                ("(", theme.bracket),
                ("define ", theme.keyword),
                ("x ", theme.symbol),
                ("1", theme.literal),
                (") ", theme.bracket),
                ("; done", theme.comment),
            ]
        );
    }
}
