//! Glob parsing and matching, with `*` confined to one path component.
//!
//! A name prefix is not a namespace: `foo-*` matches `foo-bar-*`, and a glob
//! written by an agent usually works in testing because the collision needs a
//! *second* similarly-named thing to exist before it fires. The matching here
//! is deliberately plain so the audit side ([`super::family`]) can reason about
//! what a pattern takes in: `*` matches any run of characters *within* one
//! path component, `?` and `[...]` match exactly one such character.
//!
//! The literal part before the first wildcard ([`Glob::literal_prefix`]) is the
//! thing that collides with other identifiers, so it is exposed separately from
//! the pattern text.

use std::fmt;

/// The character that separates an identifier's family from what follows it
/// (`qwen3.8-27b` | `-` | `vision-1`).
pub const DEFAULT_BOUNDARY: char = '-';

/// Why a pattern could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// `[` was opened and never closed. Refused rather than matched literally:
    /// a half-written class is a typo, and a typo that silently matches
    /// nothing (or everything) is the failure mode this module exists for.
    UnterminatedClass { pattern: String, index: usize },
    /// `[]` or `[!]` with no members.
    EmptyClass { pattern: String, index: usize },
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedClass { pattern, index } => write!(
                f,
                "pattern '{pattern}' opens '[' at index {index} and never closes it"
            ),
            Self::EmptyClass { pattern, index } => {
                write!(f, "pattern '{pattern}' has an empty [] class at index {index}")
            }
        }
    }
}

impl std::error::Error for PatternError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    /// `?` — exactly one character, never `/`.
    AnyChar,
    /// `*` — any run of characters, never crossing `/`.
    AnyRun,
    /// `[abc]` / `[!abc]` — one character from (or not in) the set, never `/`.
    Class { negated: bool, chars: Vec<char> },
}

/// A parsed glob pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    pattern: String,
    tokens: Vec<Token>,
    literal_prefix: String,
}

impl Glob {
    /// Parse a pattern. `/` is never matched by a wildcard, so one pattern can
    /// be matched against a bare name or a whole path with the same meaning.
    pub fn parse(pattern: &str) -> Result<Self, PatternError> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut tokens = Vec::with_capacity(chars.len());
        let mut literal_prefix = String::new();
        let mut prefix_open = true;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '*' => {
                    prefix_open = false;
                    tokens.push(Token::AnyRun);
                }
                '?' => {
                    prefix_open = false;
                    tokens.push(Token::AnyChar);
                }
                '[' => {
                    prefix_open = false;
                    let (class, negated, end) = parse_class(&chars, i, pattern)?;
                    tokens.push(Token::Class {
                        negated,
                        chars: class,
                    });
                    i = end;
                }
                _ => tokens.push(Token::Literal(c)),
            }
            if prefix_open {
                literal_prefix.push(c);
            }
            i += 1;
        }
        Ok(Self {
            pattern: pattern.to_owned(),
            tokens,
            literal_prefix,
        })
    }

    /// The pattern text as written.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The literal text before the first wildcard — the prefix that is also a
    /// prefix of identifiers in other families.
    pub fn literal_prefix(&self) -> &str {
        &self.literal_prefix
    }

    /// True when the pattern contains no wildcard at all: such a pattern names
    /// one identifier and cannot over-reach.
    pub fn is_exact(&self) -> bool {
        self.tokens.iter().all(|t| matches!(t, Token::Literal(_)))
    }

    /// True when `subject` (a name or a path) matches the pattern.
    pub fn matches(&self, subject: &str) -> bool {
        let subject: Vec<char> = subject.chars().collect();
        match_here(&self.tokens, 0, &subject, 0)
    }
}

impl fmt::Display for Glob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

fn parse_class(chars: &[char], open: usize, pattern: &str) -> Result<(Vec<char>, bool, usize), PatternError> {
    let mut i = open + 1;
    let negated = chars.get(i) == Some(&'!');
    if negated {
        i += 1;
    }
    let mut class = Vec::new();
    while i < chars.len() && chars[i] != ']' {
        class.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() {
        return Err(PatternError::UnterminatedClass {
            pattern: pattern.to_owned(),
            index: open,
        });
    }
    if class.is_empty() {
        return Err(PatternError::EmptyClass {
            pattern: pattern.to_owned(),
            index: open,
        });
    }
    Ok((class, negated, i + 1))
}

fn token_matches(token: &Token, c: char) -> bool {
    match token {
        // A literal '/' matches only itself; wildcards never match '/'.
        Token::Literal(l) => l == &c,
        Token::AnyChar | Token::Class { .. } if c == '/' => false,
        Token::AnyChar => true,
        Token::Class { negated, chars } => {
            let held = chars.iter().any(|x| *x == c);
            if *negated {
                !held
            } else {
                held
            }
        }
        Token::AnyRun => false,
    }
}

fn match_here(tokens: &[Token], ti: usize, subject: &[char], si: usize) -> bool {
    if ti == tokens.len() {
        return si == subject.len();
    }
    if tokens[ti] == Token::AnyRun {
        // Try the empty run first, then consume one subject character at a time
        // (stopping at '/'), backing off the star at each step.
        for k in si..=subject.len() {
            if match_here(tokens, ti + 1, subject, k) {
                return true;
            }
            if k < subject.len() && subject[k] == '/' {
                return false;
            }
        }
        return false;
    }
    if si < subject.len() && token_matches(&tokens[ti], subject[si]) {
        return match_here(tokens, ti + 1, subject, si + 1);
    }
    false
}
