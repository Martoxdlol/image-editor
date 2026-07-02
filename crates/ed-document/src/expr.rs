//! Expression language (spec §8 — locked grammar).
//!
//! ```text
//! expr   := term (("+"|"-") term)*
//! term   := factor (("*"|"/"|"%") factor)*
//! factor := number | ref | "(" expr ")" | "-" factor | call
//! ref    := "$" ident ("." ident)*
//! call   := ident "(" expr ("," expr)* ")"
//! ```
//! No loops, no user functions, no side effects. Types: number, point
//! (`.x`/`.y`), color (via refs + `lerp`). Parsed and validated at input
//! time, stored as AST, pretty-printed for editing.

use ed_core::{Color, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum ExprAst {
    Number(f64),
    Ref(Vec<String>),
    Neg(Box<ExprAst>),
    BinOp { op: BinOp, lhs: Box<ExprAst>, rhs: Box<ExprAst> },
    Call { func: Func, args: Vec<ExprAst> },
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// Whitelisted functions (spec §8).
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Func {
    Min,
    Max,
    Clamp,
    Round,
    Floor,
    Ceil,
    Abs,
    Lerp,
}

impl Func {
    fn parse(name: &str) -> Option<Func> {
        Some(match name {
            "min" => Func::Min,
            "max" => Func::Max,
            "clamp" => Func::Clamp,
            "round" => Func::Round,
            "floor" => Func::Floor,
            "ceil" => Func::Ceil,
            "abs" => Func::Abs,
            "lerp" => Func::Lerp,
            _ => return None,
        })
    }

    fn name(&self) -> &'static str {
        match self {
            Func::Min => "min",
            Func::Max => "max",
            Func::Clamp => "clamp",
            Func::Round => "round",
            Func::Floor => "floor",
            Func::Ceil => "ceil",
            Func::Abs => "abs",
            Func::Lerp => "lerp",
        }
    }

    fn arity(&self) -> std::ops::RangeInclusive<usize> {
        match self {
            Func::Min | Func::Max => 2..=usize::MAX,
            Func::Clamp | Func::Lerp => 3..=3,
            Func::Round | Func::Floor | Func::Ceil | Func::Abs => 1..=1,
        }
    }
}

/// Runtime value of an expression.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ExprValue {
    Number(f64),
    Point(Vec2),
    Color(Color),
}

#[derive(Clone, PartialEq, Debug)]
pub struct ExprError(pub String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError(msg.into()))
}

// ---------------------------------------------------------------- parser

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

pub fn parse(src: &str) -> Result<ExprAst, ExprError> {
    let mut p = Parser { src: src.as_bytes(), pos: 0 };
    let ast = p.expr()?;
    p.skip_ws();
    if p.pos != p.src.len() {
        return err(format!("unexpected input at offset {}", p.pos));
    }
    Ok(ast)
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<ExprAst, ExprError> {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek() {
                Some(b'+') => BinOp::Add,
                Some(b'-') => BinOp::Sub,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            let rhs = self.term()?;
            lhs = ExprAst::BinOp { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
    }

    fn term(&mut self) -> Result<ExprAst, ExprError> {
        let mut lhs = self.factor()?;
        loop {
            let op = match self.peek() {
                Some(b'*') => BinOp::Mul,
                Some(b'/') => BinOp::Div,
                Some(b'%') => BinOp::Mod,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            let rhs = self.factor()?;
            lhs = ExprAst::BinOp { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
    }

    fn factor(&mut self) -> Result<ExprAst, ExprError> {
        match self.peek() {
            None => err("unexpected end of expression"),
            Some(b'-') => {
                self.pos += 1;
                Ok(ExprAst::Neg(Box::new(self.factor()?)))
            }
            Some(b'(') => {
                self.pos += 1;
                let inner = self.expr()?;
                if !self.eat(b')') {
                    return err("expected ')'");
                }
                Ok(inner)
            }
            Some(b'$') => {
                self.pos += 1;
                let mut path = vec![self.ident()?];
                while self.src.get(self.pos) == Some(&b'.') {
                    self.pos += 1;
                    path.push(self.ident()?);
                }
                Ok(ExprAst::Ref(path))
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                let name = self.ident()?;
                let Some(func) = Func::parse(&name) else {
                    return err(format!("unknown function '{name}'"));
                };
                if !self.eat(b'(') {
                    return err(format!("expected '(' after '{name}'"));
                }
                let mut args = vec![self.expr()?];
                while self.eat(b',') {
                    args.push(self.expr()?);
                }
                if !self.eat(b')') {
                    return err("expected ')'");
                }
                if !func.arity().contains(&args.len()) {
                    return err(format!("{name}() takes {:?} args, got {}", func.arity(), args.len()));
                }
                Ok(ExprAst::Call { func, args })
            }
            Some(c) => err(format!("unexpected character '{}'", c as char)),
        }
    }

    fn ident(&mut self) -> Result<String, ExprError> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
        {
            self.pos += 1;
        }
        if self.pos == start {
            return err("expected identifier");
        }
        Ok(std::str::from_utf8(&self.src[start..self.pos]).unwrap().to_string())
    }

    fn number(&mut self) -> Result<ExprAst, ExprError> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'.')
        {
            self.pos += 1;
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .unwrap()
            .parse::<f64>()
            .map(ExprAst::Number)
            .map_err(|_| ExprError("invalid number".into()))
    }
}

// ---------------------------------------------------------------- printer

impl ExprAst {
    /// Pretty-print for editing (spec §8: stored as AST, printed back).
    pub fn pretty(&self) -> String {
        self.print_prec(0)
    }

    fn print_prec(&self, parent: u8) -> String {
        match self {
            ExprAst::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            ExprAst::Ref(path) => format!("${}", path.join(".")),
            ExprAst::Neg(inner) => format!("-{}", inner.print_prec(2)),
            ExprAst::BinOp { op, lhs, rhs } => {
                let (prec, sym) = match op {
                    BinOp::Add => (0, "+"),
                    BinOp::Sub => (0, "-"),
                    BinOp::Mul => (1, "*"),
                    BinOp::Div => (1, "/"),
                    BinOp::Mod => (1, "%"),
                };
                let s = format!("{} {} {}", lhs.print_prec(prec), sym, rhs.print_prec(prec + 1));
                if parent > prec {
                    format!("({s})")
                } else {
                    s
                }
            }
            ExprAst::Call { func, args } => {
                let args: Vec<_> = args.iter().map(|a| a.print_prec(0)).collect();
                format!("{}({})", func.name(), args.join(", "))
            }
        }
    }

    /// All `$refs` this expression reads — the dependency edges (spec §8).
    pub fn dependencies(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        self.collect_deps(&mut out);
        out
    }

    fn collect_deps(&self, out: &mut BTreeSet<String>) {
        match self {
            ExprAst::Number(_) => {}
            ExprAst::Ref(path) => {
                out.insert(path.join("."));
            }
            ExprAst::Neg(inner) => inner.collect_deps(out),
            ExprAst::BinOp { lhs, rhs, .. } => {
                lhs.collect_deps(out);
                rhs.collect_deps(out);
            }
            ExprAst::Call { args, .. } => {
                for a in args {
                    a.collect_deps(out);
                }
            }
        }
    }

    /// Evaluate against a resolver mapping `$ref` paths to values.
    pub fn eval(
        &self,
        resolve: &dyn Fn(&[String]) -> Option<ExprValue>,
    ) -> Result<ExprValue, ExprError> {
        use ExprValue as V;
        match self {
            ExprAst::Number(n) => Ok(V::Number(*n)),
            ExprAst::Ref(path) => {
                // `.x`/`.y` on a point resolves to a number
                if path.len() >= 2 {
                    let last = path.last().unwrap().as_str();
                    if (last == "x" || last == "y") {
                        if let Some(V::Point(p)) = resolve(&path[..path.len() - 1]) {
                            return Ok(V::Number(if last == "x" { p.x } else { p.y }));
                        }
                    }
                }
                resolve(path).ok_or_else(|| ExprError(format!("unknown reference ${}", path.join("."))))
            }
            ExprAst::Neg(inner) => match inner.eval(resolve)? {
                V::Number(n) => Ok(V::Number(-n)),
                V::Point(p) => Ok(V::Point(Vec2::new(-p.x, -p.y))),
                V::Color(_) => err("cannot negate a color"),
            },
            ExprAst::BinOp { op, lhs, rhs } => {
                let l = lhs.eval(resolve)?;
                let r = rhs.eval(resolve)?;
                match (l, r) {
                    (V::Number(a), V::Number(b)) => Ok(V::Number(match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div => a / b,
                        BinOp::Mod => a.rem_euclid(b),
                    })),
                    (V::Point(a), V::Point(b)) => match op {
                        BinOp::Add => Ok(V::Point(a + b)),
                        BinOp::Sub => Ok(V::Point(a - b)),
                        _ => err("only + and - are defined on points"),
                    },
                    (V::Point(p), V::Number(s)) | (V::Number(s), V::Point(p)) => match op {
                        BinOp::Mul => Ok(V::Point(p * s)),
                        BinOp::Div => Ok(V::Point(p * (1.0 / s))),
                        _ => err("points support * and / with numbers"),
                    },
                    _ => err("type mismatch in expression"),
                }
            }
            ExprAst::Call { func, args } => {
                let vals: Result<Vec<_>, _> = args.iter().map(|a| a.eval(resolve)).collect();
                let vals = vals?;
                let num = |v: &ExprValue| -> Result<f64, ExprError> {
                    match v {
                        V::Number(n) => Ok(*n),
                        _ => err(format!("{}() expects numbers", func.name())),
                    }
                };
                match func {
                    Func::Min | Func::Max => {
                        let mut acc = num(&vals[0])?;
                        for v in &vals[1..] {
                            let n = num(v)?;
                            acc = if *func == Func::Min { acc.min(n) } else { acc.max(n) };
                        }
                        Ok(V::Number(acc))
                    }
                    Func::Clamp => {
                        Ok(V::Number(num(&vals[0])?.clamp(num(&vals[1])?, num(&vals[2])?)))
                    }
                    Func::Round => Ok(V::Number(num(&vals[0])?.round())),
                    Func::Floor => Ok(V::Number(num(&vals[0])?.floor())),
                    Func::Ceil => Ok(V::Number(num(&vals[0])?.ceil())),
                    Func::Abs => Ok(V::Number(num(&vals[0])?.abs())),
                    Func::Lerp => {
                        let t = num(&vals[2])?;
                        match (&vals[0], &vals[1]) {
                            (V::Number(a), V::Number(b)) => Ok(V::Number(a + (b - a) * t)),
                            (V::Point(a), V::Point(b)) => Ok(V::Point(a.lerp(*b, t))),
                            (V::Color(a), V::Color(b)) => Ok(V::Color(a.lerp(b, t as f32))),
                            _ => err("lerp() arguments must be two numbers, points, or colors"),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_num(src: &str) -> f64 {
        let resolver = |path: &[String]| -> Option<ExprValue> {
            match path.join(".").as_str() {
                "gridSize" => Some(ExprValue::Number(8.0)),
                "origin" => Some(ExprValue::Point(Vec2::new(10.0, 20.0))),
                "palette.accent" => Some(ExprValue::Color(Color::from_hex("#ff0000").unwrap())),
                _ => None,
            }
        };
        match parse(src).unwrap().eval(&resolver).unwrap() {
            ExprValue::Number(n) => n,
            other => panic!("expected number, got {other:?}"),
        }
    }

    #[test]
    fn precedence_and_arithmetic() {
        assert_eq!(eval_num("1 + 2 * 3"), 7.0);
        assert_eq!(eval_num("(1 + 2) * 3"), 9.0);
        assert_eq!(eval_num("10 % 3"), 1.0);
        assert_eq!(eval_num("-4 + 6"), 2.0);
        assert_eq!(eval_num("2 * -3"), -6.0);
    }

    #[test]
    fn refs_and_functions() {
        assert_eq!(eval_num("$gridSize * 4"), 32.0);
        assert_eq!(eval_num("clamp($gridSize, 0, 5)"), 5.0);
        assert_eq!(eval_num("min(3, 1, 2)"), 1.0);
        assert_eq!(eval_num("lerp(0, 10, 0.25)"), 2.5);
        assert_eq!(eval_num("$origin.x + $origin.y"), 30.0);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse("1 +").is_err());
        assert!(parse("foo(1)").is_err()); // not whitelisted
        assert!(parse("1; drop").is_err());
        assert!(parse("clamp(1, 2)").is_err()); // arity
        assert!(parse("").is_err());
    }

    #[test]
    fn pretty_print_roundtrip() {
        for src in ["1 + 2 * 3", "(1 + 2) * 3", "$gridSize * 4 - 1", "clamp($a.b, 0, 100)", "-$x + 2"] {
            let ast = parse(src).unwrap();
            let printed = ast.pretty();
            let reparsed = parse(&printed).unwrap();
            assert_eq!(ast, reparsed, "roundtrip failed for {src:?} → {printed:?}");
        }
    }

    #[test]
    fn dependencies_tracked() {
        let ast = parse("$gridSize * 2 + $palette.accent.x + min($a, $b)").unwrap();
        let deps = ast.dependencies();
        assert!(deps.contains("gridSize"));
        assert!(deps.contains("palette.accent.x"));
        assert!(deps.contains("a") && deps.contains("b"));
    }
}
