//! The text form of an expression: a lexer and a precedence-climbing parser,
//! the inverse of the printer in `expr.rs`.

use std::ops::Range;

use crate::query::check::{QueryError, closest};
use crate::query::expr::{BinOp, Expr, Func, Lit, Node, Reduce, UnOp};

/// Words of the language: a column with one of these names is written
/// `` `name` ``.
pub(crate) const KEYWORDS: [&str; 10] = [
    "and", "or", "not", "if", "then", "else", "true", "false", "inf", "nan",
];

/// The deepest nesting an expression may have: text can come from a file,
/// and parsing (and evaluating) recurse over it.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    Ident(String),
    Word(&'static str),
    Sym(&'static str),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    span: Range<usize>,
}

fn lex(src: &str) -> Result<Vec<Token>, QueryError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let err = |m: String, span: Range<usize>| QueryError::new(m).at(src, span);
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let tok = if c.is_ascii_digit()
            || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let mut float = false;
            while i < bytes.len() {
                match bytes[i] {
                    b'0'..=b'9' => i += 1,
                    b'.' if !float => {
                        float = true;
                        i += 1;
                    }
                    b'e' | b'E' => {
                        float = true;
                        i += 1;
                        if matches!(bytes.get(i), Some(b'+' | b'-')) {
                            i += 1;
                        }
                    }
                    _ => break,
                }
            }
            let text = &src[start..i];
            if float {
                let v: f64 = text
                    .parse()
                    .map_err(|_| err(format!("`{text}` is not a number"), start..i))?;
                if !v.is_finite() {
                    return Err(err(format!("`{text}` is too large for a float"), start..i));
                }
                Tok::Float(v)
            } else {
                Tok::Int(
                    text.parse().map_err(|_| {
                        err(format!("`{text}` is too large for an integer"), start..i)
                    })?,
                )
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            let word = &src[start..i];
            match KEYWORDS.iter().find(|k| **k == word) {
                Some(k) => Tok::Word(k),
                None => Tok::Ident(word.to_owned()),
            }
        } else if c == b'`' {
            let close = src[i + 1..]
                .find('`')
                .ok_or_else(|| err("`` ` `` is never closed".into(), i..i + 1))?;
            i += close + 2;
            Tok::Ident(src[start + 1..i - 1].to_owned())
        } else {
            const SYMBOLS: [&str; 20] = [
                "**", "==", "!=", "<=", ">=", "&&", "||", "??", "+", "-", "*", "/", "^", "<", ">",
                "!", "(", ")", "[", "]",
            ];
            let rest = &src[i..];
            if let Some(s) = SYMBOLS.iter().find(|s| rest.starts_with(**s)) {
                i += s.len();
                Tok::Sym(if *s == "**" { "^" } else { s })
            } else if c == b',' {
                i += 1;
                Tok::Sym(",")
            } else {
                let ch = rest.chars().next().unwrap_or('?');
                let e = err(format!("unexpected `{ch}`"), i..i + ch.len_utf8());
                return Err(e.with_hint(match ch {
                    '&' => Some("logical and is `and` (or `&&`)".into()),
                    '|' => Some("logical or is `or` (or `||`)".into()),
                    '=' => Some("compare with `==`".into()),
                    '~' => Some("logical not is `not` (or `!`)".into()),
                    _ => None,
                }));
            }
        };
        out.push(Token {
            tok,
            span: start..i,
        });
    }
    Ok(out)
}

impl Expr {
    /// Parse the text form of an expression, as [`Display`](std::fmt::Display)
    /// writes it:
    ///
    /// ```
    /// use oxiroot_dataset::query::{Expr, col};
    ///
    /// let e = Expr::parse("nMuon >= 2 and any(Muon.pt > 20)")?;
    /// assert_eq!(e, col("nMuon").ge(2) & col("Muon.pt").gt(20).any());
    /// # Ok::<(), oxiroot_dataset::query::QueryError>(())
    /// ```
    ///
    /// The language: columns (`pt`, `Muon.pt`, or `` `any name` ``),
    /// integers, floats, `true`, `false`; `+ - * / ^` (`**` too), `== != <
    /// <= > >=` (which do not chain), `and or not` (or `&& || !`); `x ?? d`
    /// for a default where `x` is missing; `if c then a else b`; functions
    /// `sqrt abs exp log log10 sin cos tan asin acos atan sinh cosh tanh
    /// floor ceil round atan2 hypot min max`; reductions over the objects of
    /// an entry `any all count sum max min mean len`; and `x[i]` (an object
    /// by index, negative from the end) or `x[mask]` (the objects a
    /// per-object bool keeps).
    ///
    /// # Errors
    /// A [`QueryError`] pointing at what is wrong.
    pub fn parse(text: &str) -> Result<Self, QueryError> {
        let toks = lex(text)?;
        let mut p = Parser {
            src: text,
            toks,
            pos: 0,
            depth: 0,
        };
        if p.toks.is_empty() {
            return Err(QueryError::new("the expression is empty").at(text, 0..0));
        }
        let e = p.expr()?;
        if let Some(t) = p.toks.get(p.pos) {
            return Err(p.err(
                format!(
                    "unexpected `{}` after a complete expression",
                    p.text(&t.span)
                ),
                t.span.clone(),
            ));
        }
        Ok(Self(e))
    }
}

struct Parser<'s> {
    src: &'s str,
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn err(&self, message: impl Into<String>, span: Range<usize>) -> QueryError {
        QueryError::new(message).at(self.src, span)
    }

    fn text(&self, span: &Range<usize>) -> &str {
        self.src.get(span.clone()).unwrap_or("")
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn span(&self) -> Range<usize> {
        self.toks
            .get(self.pos)
            .map_or(self.src.len()..self.src.len(), |t| t.span.clone())
    }

    fn is(&self, t: &Tok) -> bool {
        self.peek() == Some(t)
    }

    fn eat(&mut self, t: &Tok, what: &str) -> Result<(), QueryError> {
        if self.is(t) {
            self.pos += 1;
            Ok(())
        } else {
            let span = self.span();
            Err(match self.toks.get(self.pos) {
                Some(found) => self.err(
                    format!("expected {what}, found `{}`", self.text(&found.span)),
                    span,
                ),
                None => self.err(
                    format!("the expression ends where {what} was expected"),
                    span,
                ),
            })
        }
    }

    fn deeper<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, QueryError>,
    ) -> Result<T, QueryError> {
        if self.depth >= MAX_DEPTH {
            return Err(self.err("the expression is nested too deeply", self.span()));
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn expr(&mut self) -> Result<Node, QueryError> {
        self.deeper(|p| {
            if p.is(&Tok::Word("if")) {
                p.pos += 1;
                let c = p.expr()?;
                p.eat(&Tok::Word("then"), "`then`")?;
                let a = p.expr()?;
                p.eat(&Tok::Word("else"), "`else`")?;
                let b = p.expr()?;
                return Ok(Node::If(Box::new(c), Box::new(a), Box::new(b)));
            }
            p.or()
        })
    }

    fn or(&mut self) -> Result<Node, QueryError> {
        let mut a = self.and()?;
        while self.is(&Tok::Word("or")) || self.is(&Tok::Sym("||")) {
            self.pos += 1;
            let b = self.and()?;
            a = Node::Binary(BinOp::Or, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn and(&mut self) -> Result<Node, QueryError> {
        let mut a = self.not()?;
        while self.is(&Tok::Word("and")) || self.is(&Tok::Sym("&&")) {
            self.pos += 1;
            let b = self.not()?;
            a = Node::Binary(BinOp::And, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn not(&mut self) -> Result<Node, QueryError> {
        if self.is(&Tok::Word("not")) || self.is(&Tok::Sym("!")) {
            self.pos += 1;
            let a = self.deeper(Self::not)?;
            return Ok(Node::Unary(UnOp::Not, Box::new(a)));
        }
        self.comparison()
    }

    fn comparison(&mut self) -> Result<Node, QueryError> {
        let a = self.coalesce()?;
        let op = |t: Option<&Tok>| match t {
            Some(Tok::Sym("==")) => Some(BinOp::Eq),
            Some(Tok::Sym("!=")) => Some(BinOp::Ne),
            Some(Tok::Sym("<")) => Some(BinOp::Lt),
            Some(Tok::Sym("<=")) => Some(BinOp::Le),
            Some(Tok::Sym(">")) => Some(BinOp::Gt),
            Some(Tok::Sym(">=")) => Some(BinOp::Ge),
            _ => None,
        };
        let Some(o) = op(self.peek()) else {
            return Ok(a);
        };
        self.pos += 1;
        let b = self.coalesce()?;
        if op(self.peek()).is_some() {
            let span = self.span();
            return Err(self
                .err("comparisons do not chain", span)
                .with_hint(Some("write `a < b and b < c`".into())));
        }
        Ok(Node::Binary(o, Box::new(a), Box::new(b)))
    }

    fn coalesce(&mut self) -> Result<Node, QueryError> {
        let mut a = self.additive()?;
        while self.is(&Tok::Sym("??")) {
            self.pos += 1;
            let b = self.additive()?;
            a = Node::UnwrapOr(Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn additive(&mut self) -> Result<Node, QueryError> {
        let mut a = self.multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("+")) => BinOp::Add,
                Some(Tok::Sym("-")) => BinOp::Sub,
                _ => return Ok(a),
            };
            self.pos += 1;
            let b = self.multiplicative()?;
            a = Node::Binary(op, Box::new(a), Box::new(b));
        }
    }

    fn multiplicative(&mut self) -> Result<Node, QueryError> {
        let mut a = self.unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym("*")) => BinOp::Mul,
                Some(Tok::Sym("/")) => BinOp::Div,
                _ => return Ok(a),
            };
            self.pos += 1;
            let b = self.unary()?;
            a = Node::Binary(op, Box::new(a), Box::new(b));
        }
    }

    fn unary(&mut self) -> Result<Node, QueryError> {
        if self.is(&Tok::Sym("-")) {
            self.pos += 1;
            let a = self.deeper(Self::unary)?;
            return Ok(match a {
                Node::Lit(Lit::Int(v)) => Node::Lit(Lit::Int(v.wrapping_neg())),
                Node::Lit(Lit::Float(v)) => Node::Lit(Lit::Float(-v)),
                a => Node::Unary(UnOp::Neg, Box::new(a)),
            });
        }
        self.power()
    }

    fn power(&mut self) -> Result<Node, QueryError> {
        let a = self.postfix()?;
        if self.is(&Tok::Sym("^")) {
            self.pos += 1;
            let b = self.deeper(Self::unary)?;
            return Ok(Node::Binary(BinOp::Pow, Box::new(a), Box::new(b)));
        }
        Ok(a)
    }

    fn postfix(&mut self) -> Result<Node, QueryError> {
        let mut a = self.primary()?;
        while self.is(&Tok::Sym("[")) {
            self.pos += 1;
            let inner = self.expr()?;
            self.eat(&Tok::Sym("]"), "`]`")?;
            a = match inner {
                Node::Lit(Lit::Int(i)) => Node::At(Box::new(a), i),
                mask => Node::Keep(Box::new(a), Box::new(mask)),
            };
        }
        Ok(a)
    }

    fn primary(&mut self) -> Result<Node, QueryError> {
        let span = self.span();
        let Some(tok) = self.peek().cloned() else {
            return Err(self.err("the expression ends too early", span));
        };
        self.pos += 1;
        Ok(match tok {
            Tok::Int(v) => Node::Lit(Lit::Int(v)),
            Tok::Float(v) => Node::Lit(Lit::Float(v)),
            Tok::Word("true") => Node::Lit(Lit::Bool(true)),
            Tok::Word("false") => Node::Lit(Lit::Bool(false)),
            Tok::Word("inf") => Node::Lit(Lit::Float(f64::INFINITY)),
            Tok::Word("nan") => Node::Lit(Lit::Float(f64::NAN)),
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.eat(&Tok::Sym(")"), "`)`")?;
                e
            }
            Tok::Ident(name) if self.is(&Tok::Sym("(")) => return self.call(name, span),
            Tok::Ident(name) => Node::Col(name),
            _ => {
                return Err(self.err(
                    format!("expected a value, found `{}`", self.text(&span)),
                    span,
                ));
            }
        })
    }

    fn call(&mut self, name: String, span: Range<usize>) -> Result<Node, QueryError> {
        self.pos += 1;
        let mut args = Vec::new();
        if !self.is(&Tok::Sym(")")) {
            args.push(self.expr()?);
            while self.is(&Tok::Sym(",")) {
                self.pos += 1;
                args.push(self.expr()?);
            }
        }
        self.eat(&Tok::Sym(")"), "`,` or `)`")?;
        if args.len() == 1
            && let Some((_, r)) = Reduce::ALL.iter().find(|(n, _)| *n == name)
        {
            return Ok(Node::Reduce(*r, Box::new(args.remove(0))));
        }
        if let Some((_, f, arity)) = Func::ALL.iter().find(|(n, _, _)| *n == name) {
            if args.len() != *arity {
                return Err(self.err(
                    format!(
                        "`{name}` takes {arity} argument{}, not {}",
                        if *arity == 1 { "" } else { "s" },
                        args.len()
                    ),
                    span,
                ));
            }
            return Ok(Node::Call(*f, args));
        }
        if Reduce::ALL.iter().any(|(n, _)| *n == name) {
            return Err(self.err(
                format!("`{name}` takes 1 argument, not {}", args.len()),
                span,
            ));
        }
        let names: Vec<String> = Func::ALL
            .iter()
            .map(|(n, _, _)| (*n).to_owned())
            .chain(Reduce::ALL.iter().map(|(n, _)| (*n).to_owned()))
            .collect();
        let hint = closest(&name, names).map(|c| format!("did you mean `{c}`?"));
        Err(self
            .err(format!("unknown function `{name}`"), span)
            .with_hint(hint))
    }
}
