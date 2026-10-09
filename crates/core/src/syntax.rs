//! Lossless tokens with byte ranges, plus a recoverable scalar-expression parser.
//! Queries and unknown constructs are left to the real engine, never guessed.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub start: usize,
    pub end: usize,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Trivia,
    Ident(String),
    Reference(String),
    Number(f64),
    Text(String),
    Symbol(String),
    End,
}
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Boolean(bool),
    Ref { table: Option<String>, name: String },
    Table(String),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Error,
}
#[derive(Debug, Clone)]
pub struct Parsed {
    pub tokens: Vec<Token>,
    pub expression: Expr,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lex(text: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut out = Vec::new();
    let mut diagnostics = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let start = i;
        let c = text[i..].chars().next().unwrap();
        i += c.len_utf8();
        let kind = if c.is_whitespace() {
            while i < text.len() && text[i..].chars().next().unwrap().is_whitespace() {
                i += text[i..].chars().next().unwrap().len_utf8();
            }
            Kind::Trivia
        } else if text[start..].starts_with("//") || text[start..].starts_with("--") {
            while i < text.len() && !text[i..].starts_with('\n') {
                i += text[i..].chars().next().unwrap().len_utf8();
            }
            Kind::Trivia
        } else if text[start..].starts_with("/*") {
            if let Some(end) = text[start + 2..].find("*/") {
                i = start + 2 + end + 2;
            } else {
                i = text.len();
                diagnostics.push(Diagnostic {
                    start,
                    end: i,
                    message: "Unclosed comment".into(),
                });
            }
            Kind::Trivia
        } else if c == '\'' || c == '"' || c == '[' {
            let close = if c == '[' { ']' } else { c };
            let mut value = String::new();
            let mut closed = false;
            while i < text.len() {
                let x = text[i..].chars().next().unwrap();
                i += x.len_utf8();
                if x == close {
                    if text[i..].starts_with(close) {
                        value.push(close);
                        i += close.len_utf8();
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    value.push(x);
                }
            }
            if !closed {
                diagnostics.push(Diagnostic {
                    start,
                    end: i,
                    message: format!("Expected '{close}'"),
                });
            }
            match c {
                '[' => Kind::Reference(value),
                '\'' => Kind::Ident(value),
                _ => Kind::Text(value),
            }
        } else if c.is_ascii_digit()
            || (c == '.' && text[i..].chars().next().is_some_and(|x| x.is_ascii_digit()))
        {
            while i < text.len() && text.as_bytes()[i].is_ascii_digit() {
                i += 1;
            }
            if i < text.len() && text.as_bytes()[i] == b'.' {
                i += 1;
                while i < text.len() && text.as_bytes()[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < text.len() && matches!(text.as_bytes()[i], b'e' | b'E') {
                i += 1;
                if i < text.len() && matches!(text.as_bytes()[i], b'+' | b'-') {
                    i += 1;
                }
                while i < text.len() && text.as_bytes()[i].is_ascii_digit() {
                    i += 1;
                }
            }
            match text[start..i].parse::<f64>() {
                Ok(n) if n.is_finite() => Kind::Number(n),
                _ => {
                    diagnostics.push(Diagnostic {
                        start,
                        end: i,
                        message: "Invalid number".into(),
                    });
                    Kind::Number(0.0)
                }
            }
        } else if c.is_alphabetic() || c == '_' {
            while i < text.len() {
                let x = text[i..].chars().next().unwrap();
                if !x.is_alphanumeric() && x != '_' {
                    break;
                }
                i += x.len_utf8();
            }
            Kind::Ident(text[start..i].into())
        } else {
            for op in ["==", "<=", ">=", "<>", "&&", "||"] {
                if text[start..].starts_with(op) {
                    i = start + op.len();
                    break;
                }
            }
            Kind::Symbol(text[start..i].into())
        };
        out.push(Token {
            kind,
            start,
            end: i,
        });
    }
    out.push(Token {
        kind: Kind::End,
        start: i,
        end: i,
    });
    (out, diagnostics)
}
pub fn parse(text: &str) -> Parsed {
    let (tokens, diagnostics) = lex(text);
    let significant = tokens
        .iter()
        .filter(|t| t.kind != Kind::Trivia)
        .cloned()
        .collect();
    let mut p = Parser {
        tokens: significant,
        pos: 0,
        diagnostics,
        depth: 0,
    };
    let expression = p.expr(0);
    if p.current().kind != Kind::End {
        p.error("Unexpected token; this syntax requires the real engine");
    }
    Parsed {
        tokens,
        expression,
        diagnostics: p.diagnostics,
    }
}
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    depth: usize,
}
impl Parser {
    fn current(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }
    fn next(&mut self) -> Token {
        let t = self.current().clone();
        if t.kind != Kind::End {
            self.pos += 1;
        }
        t
    }
    fn is(&self, s: &str) -> bool {
        self.current().kind == Kind::Symbol(s.into())
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.next();
            true
        } else {
            false
        }
    }
    fn error(&mut self, s: &str) {
        let t = self.current();
        self.diagnostics.push(Diagnostic {
            start: t.start,
            end: t.end,
            message: s.into(),
        });
    }
    fn expr(&mut self, min: u8) -> Expr {
        if self.depth >= 128 {
            self.error("Expression nesting limit exceeded");
            self.next();
            return Expr::Error;
        }
        self.depth += 1;
        let token = self.next();
        let mut left = match token.kind {
            Kind::Number(n) => Expr::Number(n),
            Kind::Text(s) => Expr::Text(s),
            Kind::Reference(name) => Expr::Ref { table: None, name },
            Kind::Ident(name) => {
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.is(")") {
                        loop {
                            args.push(self.expr(0));
                            if !self.eat(",") {
                                break;
                            }
                        }
                    }
                    if !self.eat(")") {
                        self.error("Expected ')'");
                    }
                    Expr::Call(name.to_uppercase(), args)
                } else if let Kind::Reference(column) = self.current().kind.clone() {
                    self.next();
                    Expr::Ref {
                        table: Some(name),
                        name: column,
                    }
                } else if name.eq_ignore_ascii_case("true") || name.eq_ignore_ascii_case("false") {
                    Expr::Boolean(name.eq_ignore_ascii_case("true"))
                } else {
                    Expr::Table(name)
                }
            }
            Kind::Symbol(s) if s == "(" => {
                let e = self.expr(0);
                if !self.eat(")") {
                    self.error("Expected ')'");
                }
                e
            }
            Kind::Symbol(s) if s == "+" || s == "-" => Expr::Unary(s, Box::new(self.expr(7))),
            _ => {
                self.error("Expected an expression");
                Expr::Error
            }
        };
        while let Kind::Symbol(s) = &self.current().kind {
            let op = s.clone();
            let precedence = match op.as_str() {
                "||" => 1,
                "&&" => 2,
                "=" | "==" | "<>" | "<" | ">" | "<=" | ">=" => 3,
                "&" => 4,
                "+" | "-" => 5,
                "*" | "/" => 6,
                "^" => 7,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.next();
            let right = self.expr(precedence + 1);
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        self.depth -= 1;
        left
    }
}
pub fn format(text: &str) -> String {
    // Conservative formatting: do not discard comments or change strings.
    let (tokens, diagnostics) = lex(text);
    if !diagnostics.is_empty() {
        return text.into();
    }
    let mut result = String::new();
    for t in tokens {
        match t.kind {
            Kind::Ident(ref name) if text[t.end..].trim_start().starts_with('(') => {
                result.push_str(&name.to_uppercase())
            }
            Kind::End => {}
            _ => result.push_str(&text[t.start..t.end]),
        }
    }
    result
}
