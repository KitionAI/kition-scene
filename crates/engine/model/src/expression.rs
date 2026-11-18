//! The small expression language of scripted drivers: the arithmetic subset of Python the
//! reference application evaluates without Python ("simple expressions"). Numbers, names
//! (variables, `frame`, `pi`, `e`, `True`, `False`), `+ - * / // % **`, comparisons, `not`,
//! `and`, `or`, `a if condition else b`, parentheses, and functions: `abs fabs min max floor
//! ceil trunc round int sqrt pow exp log log10 log2 sin cos tan asin acos atan atan2 sinh cosh
//! tanh radians degrees fmod hypot copysign clamp lerp smoothstep signum`.
//!
//! An expression is parsed once ([`parse`]) and evaluated against variables ([`Expression::eval`]);
//! nothing in it can loop, allocate without bound, or reach outside its numbers.

use std::collections::BTreeMap;

/// The longest expression accepted, in bytes.
pub const MAX_LENGTH: usize = 1024;
/// The deepest nesting accepted.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Number(f64),
    Name(String),
    Unary(char, Box<Node>),
    Not(Box<Node>),
    Binary(Op, Box<Node>, Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    If { then: Box<Node>, condition: Box<Node>, otherwise: Box<Node> },
    Call(String, Vec<Node>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDivide,
    Modulo,
    Power,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    root: Node,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Symbol(&'static str),
}

const SYMBOLS: [&str; 17] = ["**", "//", "<=", ">=", "==", "!=", "+", "-", "*", "/", "%", "<", ">", "(", ")", ",", "."];

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes.get(at).copied().unwrap_or(b' ');
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if byte.is_ascii_digit() || (byte == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)) {
            let start = at;
            while bytes.get(at).is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
                at += 1;
            }
            if bytes.get(at).is_some_and(|b| *b == b'e' || *b == b'E') {
                let mark = at;
                at += 1;
                if bytes.get(at).is_some_and(|b| *b == b'+' || *b == b'-') {
                    at += 1;
                }
                if bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                        at += 1;
                    }
                } else {
                    at = mark;
                }
            }
            let piece = text.get(start..at).unwrap_or("");
            let value = piece.parse::<f64>().map_err(|_| format!("`{piece}` is not a number"))?;
            out.push(Token::Number(value));
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = at;
            while bytes.get(at).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
                at += 1;
            }
            out.push(Token::Name(text.get(start..at).unwrap_or("").to_string()));
            continue;
        }
        let rest = text.get(at..).unwrap_or("");
        match SYMBOLS.iter().find(|symbol| rest.starts_with(**symbol)) {
            Some(symbol) if *symbol != "." => {
                out.push(Token::Symbol(symbol));
                at += symbol.len();
            }
            _ => return Err(format!("unexpected `{}`", rest.chars().next().unwrap_or(' '))),
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn symbol(&mut self, symbol: &str) -> bool {
        if matches!(self.peek(), Some(Token::Symbol(found)) if *found == symbol) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, word: &str) -> bool {
        if matches!(self.peek(), Some(Token::Name(found)) if found == word) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn deeper(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err("the expression nests too deeply".into()) } else { Ok(()) }
    }

    /// `or_test ['if' or_test 'else' conditional]`
    fn conditional(&mut self) -> Result<Node, String> {
        self.deeper()?;
        let then = self.or_test()?;
        let node = if self.word("if") {
            let condition = self.or_test()?;
            if !self.word("else") {
                return Err("`if` needs an `else`".into());
            }
            let otherwise = self.conditional()?;
            Node::If { then: Box::new(then), condition: Box::new(condition), otherwise: Box::new(otherwise) }
        } else {
            then
        };
        self.depth -= 1;
        Ok(node)
    }

    fn or_test(&mut self) -> Result<Node, String> {
        let mut node = self.and_test()?;
        while self.word("or") {
            node = Node::Or(Box::new(node), Box::new(self.and_test()?));
        }
        Ok(node)
    }

    fn and_test(&mut self) -> Result<Node, String> {
        let mut node = self.not_test()?;
        while self.word("and") {
            node = Node::And(Box::new(node), Box::new(self.not_test()?));
        }
        Ok(node)
    }

    fn not_test(&mut self) -> Result<Node, String> {
        if self.word("not") {
            self.deeper()?;
            let inner = self.not_test()?;
            self.depth -= 1;
            return Ok(Node::Not(Box::new(inner)));
        }
        self.comparison()
    }

    /// Comparisons chain as in Python: `a < b < c` is `a < b and b < c`.
    fn comparison(&mut self) -> Result<Node, String> {
        let first = self.sum()?;
        let mut parts: Vec<(Op, Node)> = Vec::new();
        loop {
            let op = [("<=", Op::LessEqual), (">=", Op::GreaterEqual), ("==", Op::Equal), ("!=", Op::NotEqual), ("<", Op::Less), (">", Op::Greater)]
                .into_iter()
                .find(|(symbol, _)| self.symbol(symbol));
            match op {
                Some((_, op)) => parts.push((op, self.sum()?)),
                None => break,
            }
        }
        let mut left = first;
        let mut out: Option<Node> = None;
        for (op, right) in parts {
            let test = Node::Binary(op, Box::new(left), Box::new(right.clone()));
            out = Some(match out {
                None => test,
                Some(previous) => Node::And(Box::new(previous), Box::new(test)),
            });
            left = right;
        }
        Ok(out.unwrap_or(left))
    }

    fn sum(&mut self) -> Result<Node, String> {
        let mut node = self.term()?;
        loop {
            let op = if self.symbol("+") {
                Op::Add
            } else if self.symbol("-") {
                Op::Subtract
            } else {
                break;
            };
            node = Node::Binary(op, Box::new(node), Box::new(self.term()?));
        }
        Ok(node)
    }

    fn term(&mut self) -> Result<Node, String> {
        let mut node = self.factor()?;
        loop {
            let op = if self.symbol("*") {
                Op::Multiply
            } else if self.symbol("//") {
                Op::FloorDivide
            } else if self.symbol("/") {
                Op::Divide
            } else if self.symbol("%") {
                Op::Modulo
            } else {
                break;
            };
            node = Node::Binary(op, Box::new(node), Box::new(self.factor()?));
        }
        Ok(node)
    }

    fn factor(&mut self) -> Result<Node, String> {
        for sign in ['-', '+'] {
            if self.symbol(if sign == '-' { "-" } else { "+" }) {
                self.deeper()?;
                let inner = self.factor()?;
                self.depth -= 1;
                return Ok(Node::Unary(sign, Box::new(inner)));
            }
        }
        self.power()
    }

    /// `**` binds tighter than unary minus on its left and looser on its right, as in Python.
    fn power(&mut self) -> Result<Node, String> {
        let base = self.atom()?;
        if self.symbol("**") {
            self.deeper()?;
            let exponent = self.factor()?;
            self.depth -= 1;
            return Ok(Node::Binary(Op::Power, Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Node, String> {
        match self.peek().cloned() {
            Some(Token::Number(value)) => {
                self.at += 1;
                Ok(Node::Number(value))
            }
            Some(Token::Name(name)) => {
                self.at += 1;
                if ["if", "else", "and", "or", "not"].contains(&name.as_str()) {
                    return Err(format!("unexpected `{name}`"));
                }
                if self.symbol("(") {
                    let mut arguments = Vec::new();
                    if !self.symbol(")") {
                        loop {
                            arguments.push(self.conditional()?);
                            if self.symbol(")") {
                                break;
                            }
                            if !self.symbol(",") {
                                return Err(format!("`{name}(` needs `,` or `)`"));
                            }
                        }
                    }
                    check_call(&name, arguments.len())?;
                    return Ok(Node::Call(name, arguments));
                }
                Ok(Node::Name(name))
            }
            Some(Token::Symbol("(")) => {
                self.at += 1;
                let inner = self.conditional()?;
                if !self.symbol(")") {
                    return Err("a `(` is not closed".into());
                }
                Ok(inner)
            }
            Some(Token::Symbol(symbol)) => Err(format!("unexpected `{symbol}`")),
            None => Err("the expression ends too soon".into()),
        }
    }
}

/// The functions and how many arguments each takes.
const FUNCTIONS: &[(&str, usize, usize)] = &[
    ("abs", 1, 1),
    ("fabs", 1, 1),
    ("min", 1, 64),
    ("max", 1, 64),
    ("floor", 1, 1),
    ("ceil", 1, 1),
    ("trunc", 1, 1),
    ("round", 1, 1),
    ("int", 1, 1),
    ("float", 1, 1),
    ("sqrt", 1, 1),
    ("pow", 2, 2),
    ("exp", 1, 1),
    ("log", 1, 2),
    ("log10", 1, 1),
    ("log2", 1, 1),
    ("sin", 1, 1),
    ("cos", 1, 1),
    ("tan", 1, 1),
    ("asin", 1, 1),
    ("acos", 1, 1),
    ("atan", 1, 1),
    ("atan2", 2, 2),
    ("sinh", 1, 1),
    ("cosh", 1, 1),
    ("tanh", 1, 1),
    ("radians", 1, 1),
    ("degrees", 1, 1),
    ("fmod", 2, 2),
    ("hypot", 2, 2),
    ("copysign", 2, 2),
    ("clamp", 1, 3),
    ("lerp", 3, 3),
    ("smoothstep", 3, 3),
    ("signum", 1, 1),
];

fn check_call(name: &str, count: usize) -> Result<(), String> {
    match FUNCTIONS.iter().find(|(known, _, _)| *known == name) {
        Some((_, low, high)) if (*low..=*high).contains(&count) => Ok(()),
        Some(_) => Err(format!("`{name}` does not take {count} arguments")),
        None => Err(format!("there is no function `{name}`")),
    }
}

/// Parses an expression.
pub fn parse(text: &str) -> Result<Expression, String> {
    if text.len() > MAX_LENGTH {
        return Err("the expression is too long".into());
    }
    let mut parser = Parser { tokens: tokens(text)?, at: 0, depth: 0 };
    let root = parser.conditional()?;
    if let Some(token) = parser.peek() {
        let shown = match token {
            Token::Number(value) => value.to_string(),
            Token::Name(name) => name.clone(),
            Token::Symbol(symbol) => (*symbol).to_string(),
        };
        return Err(format!("unexpected `{shown}`"));
    }
    Ok(Expression { root })
}

fn truthy(value: f64) -> bool {
    value != 0.0 && !value.is_nan()
}

fn flag(on: bool) -> f64 {
    if on { 1.0 } else { 0.0 }
}

/// Python's rounding: halves go to the even neighbour.
fn round_half_even(value: f64) -> f64 {
    let rounded = value.round();
    if (value - value.trunc()).abs() == 0.5 { 2.0 * (value / 2.0).round() } else { rounded }
}

impl Expression {
    /// The names the expression reads (variables and constants alike).
    pub fn names(&self) -> Vec<String> {
        fn walk(node: &Node, out: &mut Vec<String>) {
            match node {
                Node::Number(_) => {}
                Node::Name(name) => {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
                Node::Unary(_, inner) | Node::Not(inner) => walk(inner, out),
                Node::Binary(_, a, b) | Node::And(a, b) | Node::Or(a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Node::If { then, condition, otherwise } => {
                    walk(then, out);
                    walk(condition, out);
                    walk(otherwise, out);
                }
                Node::Call(_, arguments) => arguments.iter().for_each(|argument| walk(argument, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// The expression's value with `variables` (and `frame`, `pi`, `e`, `True`, `False`).
    pub fn eval(&self, variables: &BTreeMap<String, f64>) -> Result<f64, String> {
        let value = eval(&self.root, variables)?;
        if value.is_finite() { Ok(value) } else { Err("the expression is not a finite number".into()) }
    }
}

fn eval(node: &Node, variables: &BTreeMap<String, f64>) -> Result<f64, String> {
    Ok(match node {
        Node::Number(value) => *value,
        Node::Name(name) => match variables.get(name) {
            Some(value) => *value,
            None => match name.as_str() {
                "pi" => std::f64::consts::PI,
                "e" => std::f64::consts::E,
                "True" => 1.0,
                "False" => 0.0,
                _ => return Err(format!("there is no variable `{name}`")),
            },
        },
        Node::Unary(sign, inner) => {
            let value = eval(inner, variables)?;
            if *sign == '-' { -value } else { value }
        }
        Node::Not(inner) => flag(!truthy(eval(inner, variables)?)),
        Node::And(a, b) => {
            let left = eval(a, variables)?;
            if truthy(left) { eval(b, variables)? } else { left }
        }
        Node::Or(a, b) => {
            let left = eval(a, variables)?;
            if truthy(left) { left } else { eval(b, variables)? }
        }
        Node::If { then, condition, otherwise } => {
            if truthy(eval(condition, variables)?) {
                eval(then, variables)?
            } else {
                eval(otherwise, variables)?
            }
        }
        Node::Binary(op, a, b) => {
            let (x, y) = (eval(a, variables)?, eval(b, variables)?);
            match op {
                Op::Add => x + y,
                Op::Subtract => x - y,
                Op::Multiply => x * y,
                Op::Divide if y == 0.0 => return Err("division by zero".into()),
                Op::Divide => x / y,
                Op::FloorDivide if y == 0.0 => return Err("division by zero".into()),
                Op::FloorDivide => (x / y).floor(),
                Op::Modulo if y == 0.0 => return Err("modulo by zero".into()),
                // Python's modulo takes the divisor's sign.
                Op::Modulo => x - y * (x / y).floor(),
                Op::Power => {
                    if x == 0.0 && y < 0.0 {
                        return Err("zero to a negative power".into());
                    }
                    x.powf(y)
                }
                Op::Less => flag(x < y),
                Op::LessEqual => flag(x <= y),
                Op::Greater => flag(x > y),
                Op::GreaterEqual => flag(x >= y),
                Op::Equal => flag(x == y),
                Op::NotEqual => flag(x != y),
            }
        }
        Node::Call(name, arguments) => {
            let values = arguments.iter().map(|argument| eval(argument, variables)).collect::<Result<Vec<f64>, String>>()?;
            let arg = |index: usize| values.get(index).copied().unwrap_or(0.0);
            let (x, y) = (arg(0), arg(1));
            let domain = |ok: bool, value: f64| if ok { Ok(value) } else { Err(format!("`{name}` is out of its domain")) };
            match name.as_str() {
                "abs" | "fabs" => x.abs(),
                "min" => values.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                "trunc" | "int" => x.trunc(),
                "float" => x,
                "round" => round_half_even(x),
                "sqrt" => domain(x >= 0.0, x.sqrt())?,
                "pow" => {
                    if x == 0.0 && y < 0.0 {
                        return Err("zero to a negative power".into());
                    }
                    x.powf(y)
                }
                "exp" => x.exp(),
                "log" if values.len() == 2 => domain(x > 0.0 && y > 0.0 && y != 1.0, x.ln() / y.ln())?,
                "log" => domain(x > 0.0, x.ln())?,
                "log10" => domain(x > 0.0, x.log10())?,
                "log2" => domain(x > 0.0, x.log2())?,
                "sin" => x.sin(),
                "cos" => x.cos(),
                "tan" => x.tan(),
                "asin" => domain((-1.0..=1.0).contains(&x), x.asin())?,
                "acos" => domain((-1.0..=1.0).contains(&x), x.acos())?,
                "atan" => x.atan(),
                "atan2" => x.atan2(y),
                "sinh" => x.sinh(),
                "cosh" => x.cosh(),
                "tanh" => x.tanh(),
                "radians" => x.to_radians(),
                "degrees" => x.to_degrees(),
                "fmod" => domain(y != 0.0, x % y)?,
                "hypot" => x.hypot(y),
                "copysign" => x.copysign(y),
                "clamp" => {
                    let (low, high) = if values.len() == 3 { (y, arg(2)) } else { (0.0, 1.0) };
                    x.max(low).min(high)
                }
                "lerp" => x + (y - x) * arg(2),
                "smoothstep" => {
                    let (edge0, edge1, value) = (x, y, arg(2));
                    let t = if edge1 != edge0 { ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0) } else { flag(value >= edge1) };
                    t * t * (3.0 - 2.0 * t)
                }
                "signum" => {
                    if x > 0.0 {
                        1.0
                    } else if x < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                _ => return Err(format!("there is no function `{name}`")),
            }
        }
    })
}

/// Whether `name` can name a driver variable: a Python identifier that is not a keyword,
/// constant, or function.
pub fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    let starts = characters.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    starts
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && name.len() <= 64
        && !["if", "else", "and", "or", "not", "pi", "e", "True", "False", "frame"].contains(&name)
        && !FUNCTIONS.iter().any(|(function, _, _)| *function == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str, variables: &[(&str, f64)]) -> f64 {
        let variables: BTreeMap<String, f64> = variables.iter().map(|(name, value)| ((*name).to_string(), *value)).collect();
        parse(text).unwrap().eval(&variables).unwrap()
    }

    #[test]
    fn arithmetic_follows_python() {
        assert_eq!(value("1 + 2 * 3", &[]), 7.0);
        assert_eq!(value("-2 ** 2", &[]), -4.0);
        assert_eq!(value("2 ** -1", &[]), 0.5);
        assert_eq!(value("2 ** 3 ** 2", &[]), 512.0);
        assert_eq!(value("-7 // 2", &[]), -4.0);
        assert_eq!(value("-7 % 3", &[]), 2.0);
        assert_eq!(value("7 % -3", &[]), -2.0);
        assert_eq!(value("round(2.5) + round(3.5)", &[]), 6.0);
        assert_eq!(value("1 < 2 < 3", &[]), 1.0);
        assert_eq!(value("1 < 3 < 2", &[]), 0.0);
        assert_eq!(value("var * 2 if var > 1 else -var", &[("var", 3.0)]), 6.0);
        assert_eq!(value("var * 2 if var > 1 else -var", &[("var", 0.5)]), -0.5);
        assert_eq!(value("max(a, b, 4) + min(a, b)", &[("a", 1.0), ("b", 2.0)]), 5.0);
        assert!((value("sin(pi / 2) + cos(0) + 1e-1", &[]) - 2.1).abs() < 1e-12);
        assert_eq!(value("not 0 and 2 or 3", &[]), 2.0);
        assert_eq!(value("clamp(1.5) + clamp(-3, -1, 1)", &[]), 0.0);
    }

    #[test]
    fn mistakes_are_reported() {
        for text in ["1 +", "(1", "foo(1)", "sin(1, 2)", "1 if 2", "a b", "import os", "x.y", "1 ; 2"] {
            assert!(parse(text).and_then(|expression| expression.eval(&BTreeMap::new())).is_err(), "{text}");
        }
        assert!(parse("1 / x").unwrap().eval(&[("x".to_string(), 0.0)].into_iter().collect()).is_err());
        assert!(parse(&"(".repeat(200)).is_err());
        assert!(valid_name("var_2") && !valid_name("2var") && !valid_name("sin") && !valid_name("frame"));
    }
}
