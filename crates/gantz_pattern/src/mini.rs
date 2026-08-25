//! Mini-notation parsing, emitting `gantz/pattern` combinator source.
//!
//! The grammar:
//!
//! - Whitespace-separated steps fill a cycle. `~` is a rest.
//! - `[a b]` nests a subsequence in one step. `[a, b]` stacks sequences.
//! - `<a b>` alternates its steps across cycles.
//! - `a*2` / `a/2` speed a step up or down.
//! - `a@3` weights a step and `_` extends the previous step.
//! - `a(3,8)` / `a(3,8,1)` applies a euclidean mask to the step.
//! - Atoms parse as numbers when possible, exact rationals like `3/4`
//!   included (a `/` directly between digits stays in the number), and
//!   as symbols otherwise.
//!
//! Parsing happens at graph compile time (see the `pmini` node), so the
//! runtime never tokenizes: [`steel_src`] returns combinator source for
//! the node's expr, or `None` for malformed input, which the node turns
//! into silence.

use num_rational::Ratio;

/// Characters that terminate a word and stand alone as tokens.
const SPECIALS: &str = "[]<>(),*/@~";

/// Characters permitted inside word tokens, beyond alphanumerics. Words
/// are emitted verbatim into steel source (numbers) or quoted as symbols,
/// so the set excludes anything that could escape a symbol or literal.
const WORD_EXTRAS: &str = "-_.#!?&=^+%~$";

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Sym(char),
}

/// Combinator source for the notation string, or `None` when malformed.
pub fn steel_src(src: &str) -> Option<String> {
    let toks = tokenize(src)?;
    let (out, rest) = seq(&toks, &[])?;
    if rest.is_empty() { Some(out) } else { None }
}

fn tokenize(src: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // A / directly between digits stays inside the number token.
        let rational_slash = c == '/'
            && word.chars().last().is_some_and(|p| p.is_ascii_digit())
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_whitespace() {
            flush(&mut word, &mut toks);
        } else if SPECIALS.contains(c) && !rational_slash {
            flush(&mut word, &mut toks);
            toks.push(Tok::Sym(c));
        } else if c.is_ascii_alphanumeric() || WORD_EXTRAS.contains(c) || rational_slash {
            word.push(c);
        } else {
            return None;
        }
        i += 1;
    }
    flush(&mut word, &mut toks);
    Some(toks)
}

fn flush(word: &mut String, toks: &mut Vec<Tok>) {
    if !word.is_empty() {
        toks.push(Tok::Word(std::mem::take(word)));
    }
}

/// Parse terms until one of `closers` (left unconsumed) or exhaustion,
/// assembling with fastcat, or timecat when weighted.
fn seq<'t>(mut toks: &'t [Tok], closers: &[char]) -> Option<(String, &'t [Tok])> {
    let mut pairs: Vec<(Ratio<i64>, String)> = Vec::new();
    loop {
        match toks.first() {
            None => break,
            Some(Tok::Sym(c)) if closers.contains(c) => break,
            Some(Tok::Word(w)) if w == "_" => {
                let last = pairs.last_mut()?;
                last.0 += Ratio::from_integer(1);
                toks = &toks[1..];
            }
            _ => {
                let (pair, rest) = term(toks)?;
                pairs.push(pair);
                toks = rest;
            }
        }
    }
    Some((assemble(pairs), toks))
}

fn assemble(pairs: Vec<(Ratio<i64>, String)>) -> String {
    let one = Ratio::from_integer(1);
    if pairs.is_empty() {
        "pat/silence".to_string()
    } else if pairs.iter().all(|(w, _)| *w == one) {
        if pairs.len() == 1 {
            pairs.into_iter().next().unwrap().1
        } else {
            let ps: Vec<String> = pairs.into_iter().map(|(_, p)| p).collect();
            format!("(pat/fastcat (list {}))", ps.join(" "))
        }
    } else {
        let ws: Vec<String> = pairs
            .into_iter()
            .map(|(w, p)| format!("(list {} {p})", ratio_lit(w)))
            .collect();
        format!("(pat/timecat (list {}))", ws.join(" "))
    }
}

fn ratio_lit(r: Ratio<i64>) -> String {
    if r.is_integer() {
        r.numer().to_string()
    } else {
        format!("{}/{}", r.numer(), r.denom())
    }
}

/// A factor with its modifiers, as a `(weight, pattern-src)` pair.
fn term(toks: &[Tok]) -> Option<((Ratio<i64>, String), &[Tok])> {
    let (mut p, mut toks) = factor(toks)?;
    let mut weight = Ratio::from_integer(1);
    loop {
        match toks.first() {
            Some(Tok::Sym('*')) => {
                let (n, rest) = number_lit(&toks[1..])?;
                p = format!("(pat/fast (pat/rationalize {n}) {p})");
                toks = rest;
            }
            Some(Tok::Sym('/')) => {
                let (n, rest) = number_lit(&toks[1..])?;
                p = format!("(pat/slow (pat/rationalize {n}) {p})");
                toks = rest;
            }
            Some(Tok::Sym('@')) => {
                let (n, rest) = ratio(&toks[1..])?;
                weight = n;
                toks = rest;
            }
            Some(Tok::Sym('(')) => {
                let (k, rest) = int(&toks[1..])?;
                let rest = expect(rest, ',')?;
                let (n, rest) = int(rest)?;
                let (r, rest) = match rest.first() {
                    Some(Tok::Sym(',')) => int(&rest[1..])?,
                    _ => (0, rest),
                };
                let rest = expect(rest, ')')?;
                p = format!("(pat/euclid-with {p} {k} {n} {r})");
                toks = rest;
            }
            _ => break,
        }
    }
    Some(((weight, p), toks))
}

fn factor(toks: &[Tok]) -> Option<(String, &[Tok])> {
    match toks.first()? {
        Tok::Sym('~') => Some(("pat/silence".to_string(), &toks[1..])),
        Tok::Sym('[') => group(&toks[1..]),
        Tok::Sym('<') => alt(&toks[1..]),
        Tok::Word(w) if w != "_" => Some((atom(w)?, &toks[1..])),
        _ => None,
    }
}

fn atom(word: &str) -> Option<String> {
    if parse_number(word).is_some() {
        Some(format!("(pat/pure {word})"))
    } else if word
        .chars()
        .all(|c| c.is_ascii_digit() || "./-".contains(c))
    {
        // Number-shaped but unparseable, e.g. a zero denominator: quoting
        // it as a symbol would emit an invalid steel literal.
        None
    } else {
        Some(format!("(pat/pure '{word})"))
    }
}

/// Comma-separated sequences until `]`, stacked when there are several.
fn group(mut toks: &[Tok]) -> Option<(String, &[Tok])> {
    let mut seqs = Vec::new();
    loop {
        let (s, rest) = seq(toks, &[',', ']'])?;
        seqs.push(s);
        match rest.first()? {
            Tok::Sym(',') => toks = &rest[1..],
            Tok::Sym(']') => {
                let out = if seqs.len() == 1 {
                    seqs.into_iter().next().unwrap()
                } else {
                    format!("(pat/stack (list {}))", seqs.join(" "))
                };
                return Some((out, &rest[1..]));
            }
            _ => return None,
        }
    }
}

/// Terms until `>`, alternated one per cycle.
fn alt(mut toks: &[Tok]) -> Option<(String, &[Tok])> {
    let mut ps = Vec::new();
    loop {
        match toks.first()? {
            Tok::Sym('>') => {
                if ps.is_empty() {
                    return None;
                }
                return Some((format!("(pat/slowcat (list {}))", ps.join(" ")), &toks[1..]));
            }
            _ => {
                let ((_, p), rest) = term(toks)?;
                ps.push(p);
                toks = rest;
            }
        }
    }
}

fn expect(toks: &[Tok], sym: char) -> Option<&[Tok]> {
    match toks.first()? {
        Tok::Sym(c) if *c == sym => Some(&toks[1..]),
        _ => None,
    }
}

/// The next token as a verbatim number literal for emission.
fn number_lit(toks: &[Tok]) -> Option<(String, &[Tok])> {
    match toks.first()? {
        Tok::Word(w) if parse_number(w).is_some() => Some((w.clone(), &toks[1..])),
        _ => None,
    }
}

/// The next token as an exact ratio (weights need arithmetic for `_`).
fn ratio(toks: &[Tok]) -> Option<(Ratio<i64>, &[Tok])> {
    match toks.first()? {
        Tok::Word(w) => Some((parse_number(w)?, &toks[1..])),
        _ => None,
    }
}

/// The next token as an integer (euclid parameters).
fn int(toks: &[Tok]) -> Option<(i64, &[Tok])> {
    let (r, rest) = ratio(toks)?;
    if r.is_integer() {
        Some((r.to_integer(), rest))
    } else {
        None
    }
}

/// Parse a word as an exact ratio: an integer, an `n/d` rational, or a
/// float snapped to the 1/1920 grid (mirroring `pat/rationalize`).
fn parse_number(word: &str) -> Option<Ratio<i64>> {
    if let Ok(i) = word.parse::<i64>() {
        return Some(Ratio::from_integer(i));
    }
    if let Some((n, d)) = word.split_once('/') {
        let (n, d) = (n.parse::<i64>().ok()?, d.parse::<i64>().ok()?);
        if d == 0 {
            return None;
        }
        return Some(Ratio::new(n, d));
    }
    let f = word.parse::<f64>().ok()?;
    if !f.is_finite() {
        return None;
    }
    Some(Ratio::new((f * 1920.0).round() as i64, 1920))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emission_shapes() {
        assert_eq!(steel_src("bd").unwrap(), "(pat/pure 'bd)");
        assert_eq!(
            steel_src("bd sn").unwrap(),
            "(pat/fastcat (list (pat/pure 'bd) (pat/pure 'sn)))",
        );
        assert_eq!(steel_src("~").unwrap(), "pat/silence");
        assert_eq!(steel_src("0.5").unwrap(), "(pat/pure 0.5)");
        assert_eq!(steel_src("3/4").unwrap(), "(pat/pure 3/4)");
        assert_eq!(
            steel_src("bd*2").unwrap(),
            "(pat/fast (pat/rationalize 2) (pat/pure 'bd))",
        );
        assert_eq!(
            steel_src("[a b]/2").unwrap(),
            "(pat/slow (pat/rationalize 2) (pat/fastcat (list (pat/pure 'a) (pat/pure 'b))))",
        );
        assert_eq!(
            steel_src("<c d>").unwrap(),
            "(pat/slowcat (list (pat/pure 'c) (pat/pure 'd)))",
        );
        assert_eq!(
            steel_src("[a, b]").unwrap(),
            "(pat/stack (list (pat/pure 'a) (pat/pure 'b)))",
        );
        assert_eq!(
            steel_src("a@3 b").unwrap(),
            "(pat/timecat (list (list 3 (pat/pure 'a)) (list 1 (pat/pure 'b))))",
        );
        assert_eq!(steel_src("a@3 b").unwrap(), steel_src("a _ _ b").unwrap());
        assert_eq!(
            steel_src("bd(3,8)").unwrap(),
            "(pat/euclid-with (pat/pure 'bd) 3 8 0)",
        );
        assert_eq!(
            steel_src("bd(3,8,1)").unwrap(),
            "(pat/euclid-with (pat/pure 'bd) 3 8 1)",
        );
        // Fractional weights stay exact.
        assert_eq!(
            steel_src("a@1/2 b").unwrap(),
            "(pat/timecat (list (list 1/2 (pat/pure 'a)) (list 1 (pat/pure 'b))))",
        );
    }

    #[test]
    fn malformed_is_none() {
        for src in [
            "bd [",
            "*2",
            "<a",
            "bd(3",
            ")",
            "_ bd",
            "<>",
            "bd(3.5,8)",
            "1/0",
            "bd\"quote",
            "bd(3,8",
            "[a,",
        ] {
            assert!(steel_src(src).is_none(), "{src:?} should fail");
        }
        // Empty and whitespace-only inputs are silence, not failures.
        assert_eq!(steel_src("").unwrap(), "pat/silence");
        assert_eq!(steel_src("   ").unwrap(), "pat/silence");
    }
}
