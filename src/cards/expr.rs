use std::{cmp::Ordering, sync::LazyLock};

use anyhow::{bail, ensure, Context, Result};
use regex::Regex;
use serde_json::{Map, Value};

use super::RepoFingerprint;

type Fields = Map<String, Value>;

pub(super) enum Expr {
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
    Bool(Operand),
    Compare(Operand, OrderingTest, Operand),
}

pub(super) enum Operand {
    Field(String),
    Literal(Value),
}

#[derive(Clone, Copy)]
pub(super) enum OrderingTest {
    Eq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
}

static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(&&|\|\||!=|==|>=|<=|[!()<>]|-?[0-9]+|[a-zA-Z_][a-zA-Z_0-9]*|"(?:\\.|[^"\\])*"|'[^']*')"#)
        .expect("fixed repo expression lexer")
});

impl Expr {
    pub(super) fn compile(input: &str) -> Result<Self> {
        let fields = fingerprint_fields(&RepoFingerprint::default())?;
        let mut parser = Parser {
            rest: input,
            fields: &fields,
        };
        let expr = parser.any(0)?;
        ensure!(
            parser.rest.trim().is_empty(),
            "unexpected input {:?}",
            parser.rest.trim()
        );
        Ok(expr)
    }

    pub(super) fn matches(&self, fields: &Fields) -> bool {
        match self {
            Self::All(parts) => parts.iter().all(|expr| expr.matches(fields)),
            Self::Any(parts) => parts.iter().any(|expr| expr.matches(fields)),
            Self::Not(expr) => !expr.matches(fields),
            Self::Bool(operand) => operand.value(fields).as_bool() == Some(true),
            Self::Compare(a, test, b) => {
                let (a, b) = (a.value(fields), b.value(fields));
                let ordering = match (a, b) {
                    (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
                    (Value::String(a), Value::String(b)) => a.cmp(b),
                    (a, b) => integer(a).cmp(&integer(b)),
                };
                match test {
                    OrderingTest::Eq => ordering == Ordering::Equal,
                    OrderingTest::NotEq => ordering != Ordering::Equal,
                    OrderingTest::Lt => ordering == Ordering::Less,
                    OrderingTest::Le => ordering != Ordering::Greater,
                    OrderingTest::Gt => ordering == Ordering::Greater,
                    OrderingTest::Ge => ordering != Ordering::Less,
                }
            }
        }
    }
}

fn integer(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

pub(super) fn fingerprint_fields(fp: &RepoFingerprint) -> Result<Fields> {
    let Value::Object(fields) = serde_json::to_value(fp)? else {
        bail!("fingerprint must serialize as an object");
    };
    Ok(fields)
}

impl Operand {
    fn value<'a>(&'a self, fields: &'a Fields) -> &'a Value {
        match self {
            Self::Field(name) => &fields[name],
            Self::Literal(value) => value,
        }
    }
}

struct Parser<'a, 'f> {
    rest: &'a str,
    fields: &'f Fields,
}

impl<'a> Parser<'a, '_> {
    fn peek(&self) -> Option<&'a str> {
        TOKEN.find(self.rest.trim_start()).map(|m| m.as_str())
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.peek() == Some(token) {
            self.rest = &self.rest.trim_start()[token.len()..];
            true
        } else {
            false
        }
    }

    fn any(&mut self, depth: usize) -> Result<Expr> {
        let first = self.all(depth)?;
        if !self.eat("||") {
            return Ok(first);
        }
        let mut parts = vec![first, self.all(depth)?];
        while self.eat("||") {
            parts.push(self.all(depth)?);
        }
        Ok(Expr::Any(parts))
    }

    fn all(&mut self, depth: usize) -> Result<Expr> {
        let first = self.term(depth)?;
        if !self.eat("&&") {
            return Ok(first);
        }
        let mut parts = vec![first, self.term(depth)?];
        while self.eat("&&") {
            parts.push(self.term(depth)?);
        }
        Ok(Expr::All(parts))
    }

    fn term(&mut self, depth: usize) -> Result<Expr> {
        ensure!(depth < 64, "repo expression nesting exceeds 64 levels");
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.term(depth + 1)?)));
        }
        if self.eat("(") {
            let expr = self.any(depth + 1)?;
            ensure!(self.eat(")"), "missing closing `)`");
            return Ok(expr);
        }
        let lhs = self.operand()?;
        let value = lhs.value(self.fields);
        let op = match self.peek() {
            Some("==") => OrderingTest::Eq,
            Some("!=") => OrderingTest::NotEq,
            Some("<") => OrderingTest::Lt,
            Some("<=") => OrderingTest::Le,
            Some(">") => OrderingTest::Gt,
            Some(">=") => OrderingTest::Ge,
            _ => {
                ensure!(
                    value.is_boolean(),
                    "non-boolean operand requires a comparison"
                );
                return Ok(Expr::Bool(lhs));
            }
        };
        if let Some(token) = self.peek() {
            self.eat(token);
        }
        let rhs = self.operand()?;
        ensure!(
            std::mem::discriminant(value) == std::mem::discriminant(rhs.value(self.fields)),
            "cannot compare operands of different types"
        );
        Ok(Expr::Compare(lhs, op, rhs))
    }

    fn operand(&mut self) -> Result<Operand> {
        let token = self
            .peek()
            .with_context(|| format!("expected operand at {:?}", self.rest.trim()))?;
        self.eat(token);
        match token.as_bytes()[0] {
            b'\'' => Ok(Operand::Literal(Value::String(
                token[1..token.len() - 1].to_owned(),
            ))),
            b'"' | b'-' | b'0'..=b'9' => {
                let value: Value = serde_json::from_str(token).context("invalid literal")?;
                ensure!(
                    value.is_string() || integer(&value).is_some(),
                    "integer out of range"
                );
                Ok(Operand::Literal(value))
            }
            _ if token == "true" || token == "false" => {
                Ok(Operand::Literal(Value::Bool(token == "true")))
            }
            _ => {
                let value = self
                    .fields
                    .get(token)
                    .with_context(|| format!("unknown repo fingerprint field `{token}`"))?;
                ensure!(
                    value.is_boolean() || value.is_number() || value.is_string(),
                    "`{token}` is not a scalar field"
                );
                Ok(Operand::Field(token.to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_are_typed_and_validate_both_sides() {
        let fp = RepoFingerprint {
            has_rust: true,
            rust_files: u64::MAX,
            ..RepoFingerprint::default()
        };
        let fields = fingerprint_fields(&fp).unwrap();
        for (expr, expected) in [
            ("false || has_rust && !has_nix", true),
            ("!(false || has_rust) && has_nix", false),
            ("rust_files > 9223372036854775807 && rust_loc >= 0", true),
            ("forge == 'none' && rust_files != 0 && rust_loc <= 0", true),
            ("rust_files < 2 || has_rust == false", false),
        ] {
            assert_eq!(
                Expr::compile(expr).unwrap().matches(&fields),
                expected,
                "{expr}"
            );
        }
        for expr in [
            "true || invalid",
            "rust_files == true",
            "languages",
            "(has_rust",
            "has_rust &&",
            "has_rust 🦀",
        ] {
            assert!(Expr::compile(expr).is_err(), "{expr}");
        }
    }
}
