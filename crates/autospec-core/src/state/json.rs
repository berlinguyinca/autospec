use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) enum JsonValue {
    Null,
    Number(u64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub(crate) fn into_object(self, context: &str) -> Result<BTreeMap<String, Self>, String> {
        match self {
            Self::Object(value) => Ok(value),
            _ => Err(format!("{context} must be a JSON object")),
        }
    }

    pub(crate) fn into_array(self, context: &str) -> Result<Vec<Self>, String> {
        match self {
            Self::Array(value) => Ok(value),
            _ => Err(format!("{context} must be a JSON array")),
        }
    }

    pub(crate) fn into_number(self, context: &str) -> Result<u64, String> {
        match self {
            Self::Number(value) => Ok(value),
            _ => Err(format!("{context} must be a JSON number")),
        }
    }

    pub(crate) fn into_string(self, context: &str) -> Result<String, String> {
        match self {
            Self::String(value) => Ok(value),
            _ => Err(format!("{context} must be a JSON string")),
        }
    }

    pub(crate) fn into_optional_string(self, context: &str) -> Result<Option<String>, String> {
        match self {
            Self::Null => Ok(None),
            Self::String(value) => Ok(Some(value)),
            _ => Err(format!("{context} must be a JSON string or null")),
        }
    }
}

pub(crate) struct JsonParser<'a> {
    input: &'a str,
    index: usize,
}

impl<'a> JsonParser<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        Self { input, index: 0 }
    }

    pub(crate) fn parse(mut self) -> Result<JsonValue, String> {
        let value = self.parse_value()?;
        self.skip_whitespace();
        if self.index != self.input.len() {
            return Err(format!("unexpected trailing JSON at byte {}", self.index));
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_whitespace();
        match self.peek_byte() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(JsonValue::String),
            Some(b'n') => self.parse_null(),
            Some(byte) if byte.is_ascii_digit() => self.parse_number(),
            Some(_) => Err(format!("unexpected JSON token at byte {}", self.index)),
            None => Err("unexpected end of JSON input".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect_byte(b'{')?;
        self.skip_whitespace();
        let mut values = BTreeMap::new();
        if self.consume_byte(b'}') {
            return Ok(JsonValue::Object(values));
        }

        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            let value = self.parse_value()?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!("duplicate JSON object key: {key}"));
            }
            self.skip_whitespace();
            if self.consume_byte(b'}') {
                return Ok(JsonValue::Object(values));
            }
            self.expect_byte(b',')?;
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect_byte(b'[')?;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.consume_byte(b']') {
            return Ok(JsonValue::Array(values));
        }

        loop {
            values.push(self.parse_value()?);
            self.skip_whitespace();
            if self.consume_byte(b']') {
                return Ok(JsonValue::Array(values));
            }
            self.expect_byte(b',')?;
        }
    }

    fn parse_null(&mut self) -> Result<JsonValue, String> {
        self.consume_literal("null")?;
        Ok(JsonValue::Null)
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.index;
        if self.consume_byte(b'0') {
            if self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(format!("leading zero in JSON number at byte {start}"));
            }
        } else {
            while self.peek_byte().is_some_and(|byte| byte.is_ascii_digit()) {
                self.index += 1;
            }
        }
        let number = self.input[start..self.index]
            .parse::<u64>()
            .map_err(|error| format!("invalid JSON number at byte {start}: {error}"))?;
        Ok(JsonValue::Number(number))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect_byte(b'"')?;
        let mut value = String::new();
        loop {
            let character = self
                .next_character()
                .ok_or_else(|| "unterminated JSON string".to_string())?;
            match character {
                '"' => return Ok(value),
                '\\' => value.push(self.parse_escape_sequence()?),
                character if character.is_control() => {
                    return Err(format!(
                        "control character in JSON string at byte {}",
                        self.index
                    ))
                }
                character => value.push(character),
            }
        }
    }

    fn parse_escape_sequence(&mut self) -> Result<char, String> {
        match self
            .next_character()
            .ok_or_else(|| "unterminated JSON escape sequence".to_string())?
        {
            '"' => Ok('"'),
            '\\' => Ok('\\'),
            '/' => Ok('/'),
            'b' => Ok('\u{08}'),
            'f' => Ok('\u{0C}'),
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'u' => self.parse_unicode_escape(),
            _ => Err(format!(
                "invalid JSON escape sequence at byte {}",
                self.index
            )),
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let first = self.parse_hex_code_unit()?;
        if (0xD800..=0xDBFF).contains(&first) {
            if !self.consume_byte(b'\\') || !self.consume_byte(b'u') {
                return Err(
                    "high surrogate must be followed immediately by a low surrogate".to_string(),
                );
            }
            let second = self.parse_hex_code_unit()?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err("high surrogate must be followed by a low surrogate".to_string());
            }
            let code_point = 0x10000 + (((first as u32 - 0xD800) << 10) | (second as u32 - 0xDC00));
            return char::from_u32(code_point)
                .ok_or_else(|| "invalid surrogate pair in JSON string".to_string());
        }
        if (0xDC00..=0xDFFF).contains(&first) {
            return Err("unexpected low surrogate in JSON string".to_string());
        }
        char::from_u32(first as u32)
            .ok_or_else(|| "invalid Unicode escape in JSON string".to_string())
    }

    fn parse_hex_code_unit(&mut self) -> Result<u16, String> {
        let start = self.index;
        let end = start
            .checked_add(4)
            .ok_or_else(|| "invalid JSON escape index".to_string())?;
        let value = self
            .input
            .get(start..end)
            .ok_or_else(|| "incomplete Unicode escape in JSON string".to_string())?;
        if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("invalid Unicode escape at byte {start}"));
        }
        self.index = end;
        u16::from_str_radix(value, 16)
            .map_err(|error| format!("invalid Unicode escape at byte {start}: {error}"))
    }

    fn consume_literal(&mut self, literal: &str) -> Result<(), String> {
        if self.input[self.index..].starts_with(literal) {
            self.index += literal.len();
            Ok(())
        } else {
            Err(format!("expected {literal} at byte {}", self.index))
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek_byte()
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.index += 1;
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), String> {
        self.skip_whitespace();
        if self.consume_byte(expected) {
            Ok(())
        } else {
            Err(format!(
                "expected {} at byte {}",
                expected as char, self.index
            ))
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.peek_byte() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.as_bytes().get(self.index).copied()
    }

    fn next_character(&mut self) -> Option<char> {
        let character = self.input[self.index..].chars().next()?;
        self.index += character.len_utf8();
        Some(character)
    }
}
