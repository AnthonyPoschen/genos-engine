//! A small JSON reader and writer: MCP messages, glTF documents and debug replies.
//!
//! Objects keep their key order. Numbers that fit in `i64` stay integers so a
//! JSON-RPC id round-trips.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        let Value::Object(pairs) = self else {
            return None;
        };
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(Number::Int(value)) => Some(*value),
            Value::Number(Number::Float(value))
                if value.fract() == 0.0
                    && *value >= i64::MIN as f64
                    && *value <= i64::MAX as f64 =>
            {
                Some(*value as i64)
            }
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_i64().and_then(|value| u64::try_from(value).ok())
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(Number::Int(value)) => Some(*value as f64),
            Value::Number(Number::Float(value)) if value.is_finite() => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

pub fn string(text: impl Into<String>) -> Value {
    Value::String(text.into())
}

pub fn int(value: i64) -> Value {
    Value::Number(Number::Int(value))
}

pub fn float(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        Value::Number(Number::Int(value as i64))
    } else {
        Value::Number(Number::Float(value))
    }
}

pub fn bool(value: bool) -> Value {
    Value::Bool(value)
}

pub fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

pub fn array(items: Vec<Value>) -> Value {
    Value::Array(items)
}

pub fn parse(text: &str) -> Result<Value, String> {
    let mut parser = Parser { text, index: 0 };
    let value = parser.value()?;
    parser.skip();
    if parser.index != parser.text.len() {
        return Err("trailing json".into());
    }
    Ok(value)
}

pub fn encode(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(Number::Int(value)) => {
            let _ = write!(out, "{value}");
        }
        Value::Number(Number::Float(value)) => write_float(out, *value),
        Value::String(text) => write_string(out, text),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(pairs) => {
            out.push('{');
            for (index, (key, item)) in pairs.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, item);
            }
            out.push('}');
        }
    }
}

fn write_float(out: &mut String, value: f64) {
    if !value.is_finite() {
        out.push_str("null");
        return;
    }
    let text = format!("{value}");
    if text == "inf" || text == "-inf" || text == "NaN" {
        out.push_str("null");
        return;
    }
    out.push_str(&text);
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    text: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn value(&mut self) -> Result<Value, String> {
        self.skip();
        let Some(ch) = self.peek() else {
            return Err("unexpected end".into());
        };
        match ch {
            'n' => self.literal("null", Value::Null),
            't' => self.literal("true", Value::Bool(true)),
            'f' => self.literal("false", Value::Bool(false)),
            '"' => Ok(Value::String(self.string()?)),
            '[' => self.array(),
            '{' => self.object(),
            '-' | '0'..='9' => Ok(Value::Number(self.number()?)),
            _ => Err("invalid json".into()),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.text[self.index..].starts_with(word) {
            self.index += word.len();
            Ok(value)
        } else {
            Err("invalid json".into())
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.bump();
        self.skip();
        let mut items = Vec::new();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(']') => {
                    self.bump();
                    break;
                }
                _ => return Err("invalid array".into()),
            }
        }
        Ok(Value::Array(items))
    }

    fn object(&mut self) -> Result<Value, String> {
        self.bump();
        self.skip();
        let mut pairs = Vec::new();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Object(pairs));
        }
        loop {
            self.skip();
            if self.peek() != Some('"') {
                return Err("invalid object".into());
            }
            let key = self.string()?;
            self.skip();
            if self.peek() != Some(':') {
                return Err("invalid object".into());
            }
            self.bump();
            let value = self.value()?;
            pairs.push((key, value));
            self.skip();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    break;
                }
                _ => return Err("invalid object".into()),
            }
        }
        Ok(Value::Object(pairs))
    }

    fn string(&mut self) -> Result<String, String> {
        self.bump();
        let mut out = String::new();
        loop {
            let Some(ch) = self.bump_char() else {
                return Err("unterminated string".into());
            };
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let Some(escaped) = self.bump_char() else {
                        return Err("bad escape".into());
                    };
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                let Some(digit) = self.bump_char() else {
                                    return Err("bad unicode".into());
                                };
                                hex.push(digit);
                            }
                            let code = u32::from_str_radix(&hex, 16).map_err(|_| "bad unicode")?;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err("bad escape".into()),
                    }
                }
                ch if (ch as u32) < 0x20 => return Err("raw control in string".into()),
                ch => out.push(ch),
            }
        }
    }

    fn number(&mut self) -> Result<Number, String> {
        let start = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        match self.peek() {
            Some('0') => {
                self.bump();
            }
            Some('1'..='9') => {
                while matches!(self.peek(), Some('0'..='9')) {
                    self.bump();
                }
            }
            _ => return Err("bad number".into()),
        }
        let mut fractional = false;
        if self.peek() == Some('.') {
            fractional = true;
            self.bump();
            let mark = self.index;
            while matches!(self.peek(), Some('0'..='9')) {
                self.bump();
            }
            if self.index == mark {
                return Err("bad number".into());
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            fractional = true;
            self.bump();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.bump();
            }
            let mark = self.index;
            while matches!(self.peek(), Some('0'..='9')) {
                self.bump();
            }
            if self.index == mark {
                return Err("bad number".into());
            }
        }
        let text = &self.text[start..self.index];
        if !fractional {
            if let Ok(value) = text.parse::<i64>() {
                return Ok(Number::Int(value));
            }
        }
        let value = text.parse::<f64>().map_err(|_| "bad number")?;
        if !value.is_finite() {
            return Err("bad number".into());
        }
        Ok(Number::Float(value))
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.index..].chars().next()
    }

    fn bump(&mut self) {
        if let Some(ch) = self.peek() {
            self.index += ch.len_utf8();
        }
    }

    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }
}
