use std::ffi::{OsStr, OsString};
use std::iter::Peekable;
use std::str::Chars;

use crate::runtime_env::IsolationDiagnostic;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MavenArgs {
    pub(super) tokens: Vec<OsString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MavenArgPlatform {
    Posix,
    Windows,
}

impl MavenArgPlatform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

impl MavenArgs {
    pub fn parse(source: &str) -> Result<Self, IsolationDiagnostic> {
        Self::parse_for(source, MavenArgPlatform::current())
    }

    pub fn parse_for(
        source: &str,
        platform: MavenArgPlatform,
    ) -> Result<Self, IsolationDiagnostic> {
        let tokens = match platform {
            MavenArgPlatform::Posix => PosixParser::new(source).parse()?,
            MavenArgPlatform::Windows => parse_windows(source)?,
        };
        Ok(Self { tokens })
    }

    pub fn from_tokens<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            tokens: tokens.into_iter().map(Into::into).collect(),
        }
    }

    pub fn tokens(&self) -> &[OsString] {
        &self.tokens
    }

    pub fn append_property(&mut self, key: &str, value: &str) {
        self.tokens.push(OsString::from(format!("-D{key}={value}")));
    }

    pub fn render(&self) -> Result<String, IsolationDiagnostic> {
        self.render_for(MavenArgPlatform::current())
    }

    pub fn render_for(&self, platform: MavenArgPlatform) -> Result<String, IsolationDiagnostic> {
        Ok(self
            .tokens
            .iter()
            .map(|token| match platform {
                MavenArgPlatform::Posix => Ok(quote_posix(token.as_os_str())),
                MavenArgPlatform::Windows => quote_windows(token.as_os_str()),
            })
            .collect::<Result<Vec<_>, _>>()?
            .join(" "))
    }
}

struct PosixParser<'a> {
    characters: Peekable<Chars<'a>>,
    tokens: Vec<OsString>,
    token: String,
    quote: Option<char>,
    started: bool,
}

impl<'a> PosixParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            characters: source.chars().peekable(),
            tokens: Vec::new(),
            token: String::new(),
            quote: None,
            started: false,
        }
    }

    fn parse(mut self) -> Result<Vec<OsString>, IsolationDiagnostic> {
        while let Some(character) = self.characters.next() {
            self.consume(character)?;
        }
        if self.quote.is_some() {
            return Err(parse_diagnostic("unterminated quote in MAVEN_ARGS"));
        }
        self.flush();
        Ok(self.tokens)
    }

    fn consume(&mut self, character: char) -> Result<(), IsolationDiagnostic> {
        match self.quote {
            Some(marker) if character == marker => self.quote = None,
            Some('\'') => self.token.push(character),
            Some('"') if character == '\\' => self.consume_double_escape(),
            Some(_) => self.token.push(character),
            None if character == '\'' || character == '"' => self.open_quote(character),
            None if character.is_whitespace() => self.flush(),
            None if character == '\\' => self.consume_escape()?,
            None => self.push(character),
        }
        Ok(())
    }

    fn consume_double_escape(&mut self) {
        match self.characters.peek().copied() {
            Some('\n') => {
                self.characters.next();
            }
            Some(next) if matches!(next, '"' | '\\' | '$' | '`') => {
                self.characters.next();
                self.token.push(next);
            }
            _ => self.token.push('\\'),
        }
    }

    fn consume_escape(&mut self) -> Result<(), IsolationDiagnostic> {
        let next = self
            .characters
            .next()
            .ok_or_else(|| parse_diagnostic("trailing escape in MAVEN_ARGS"))?;
        self.push(next);
        Ok(())
    }

    fn open_quote(&mut self, marker: char) {
        self.quote = Some(marker);
        self.started = true;
    }

    fn push(&mut self, character: char) {
        self.token.push(character);
        self.started = true;
    }

    fn flush(&mut self) {
        if self.started {
            self.tokens
                .push(OsString::from(std::mem::take(&mut self.token)));
            self.started = false;
        }
    }
}

fn parse_windows(source: &str) -> Result<Vec<OsString>, IsolationDiagnostic> {
    let characters = source.chars().collect::<Vec<_>>();
    let mut index = 0;
    let mut tokens = Vec::new();
    while index < characters.len() {
        while characters
            .get(index)
            .is_some_and(|value| value.is_whitespace())
        {
            index += 1;
        }
        if index < characters.len() {
            tokens.push(OsString::from(parse_windows_token(
                &characters,
                &mut index,
            )?));
        }
    }
    Ok(tokens)
}

fn parse_windows_token(
    characters: &[char],
    index: &mut usize,
) -> Result<String, IsolationDiagnostic> {
    let mut token = String::new();
    let mut quoted = false;
    while *index < characters.len() {
        if characters[*index].is_whitespace() && !quoted {
            break;
        }
        let slashes = count_backslashes(characters, index);
        if characters.get(*index) == Some(&'"') {
            token.extend(std::iter::repeat_n('\\', slashes / 2));
            if slashes.is_multiple_of(2) {
                quoted = !quoted;
            } else {
                token.push('"');
            }
            *index += 1;
        } else {
            token.extend(std::iter::repeat_n('\\', slashes));
            if let Some(character) = characters.get(*index) {
                token.push(*character);
                *index += 1;
            }
        }
    }
    if quoted {
        return Err(parse_diagnostic("unterminated quote in MAVEN_ARGS"));
    }
    Ok(token)
}

fn count_backslashes(characters: &[char], index: &mut usize) -> usize {
    let start = *index;
    while characters.get(*index) == Some(&'\\') {
        *index += 1;
    }
    *index - start
}

fn quote_posix(token: &OsStr) -> String {
    let value = token.to_string_lossy();
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:=+".contains(character))
    {
        value.into_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn quote_windows(token: &OsStr) -> Result<String, IsolationDiagnostic> {
    let value = token.to_string_lossy();
    if value
        .chars()
        .any(|character| matches!(character, '%' | '!'))
    {
        return Err(diagnostic(
            "MAVEN_ARGUMENT_UNSAFE_WINDOWS_EXPANSION",
            "MAVEN_ARGS contains '%' or '!', which mvn.cmd may expand",
        ));
    }
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._/:=+\\".contains(character))
    {
        return Ok(value.into_owned());
    }
    let mut rendered = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' {
            slashes += 1;
        } else if character == '"' {
            rendered.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
            rendered.push('"');
            slashes = 0;
        } else {
            rendered.extend(std::iter::repeat_n('\\', slashes));
            rendered.push(character);
            slashes = 0;
        }
    }
    rendered.extend(std::iter::repeat_n('\\', slashes * 2));
    rendered.push('"');
    Ok(rendered)
}

fn parse_diagnostic(evidence: &str) -> IsolationDiagnostic {
    diagnostic("MAVEN_ARGUMENT_PARSE", evidence)
}

fn diagnostic(code: &str, evidence: &str) -> IsolationDiagnostic {
    super::diagnostic(code, "MAVEN_ARGS", evidence, "")
}
