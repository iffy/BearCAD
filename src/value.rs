//! Length expressions with units (SPEC §5.2–5.3).
//!
//! Canonical internal unit is millimetres. Supported length units: mm, cm, m, ft, in.
//! Bare numbers are interpreted as millimetres.

/// Evaluate a length expression to millimetres, or `None` if parsing fails.
pub fn eval_length_mm(text: &str) -> Option<f32> {
    let mut p = Parser::new(text.trim());
    let value = p.parse_expr().ok()?;
    p.skip_ws();
    if p.at_end() {
        Some(value)
    } else {
        None
    }
}

/// Whether the text uses expression syntax (operators, parentheses, or units) and
/// should show a computed value above the input field.
pub fn shows_computed_length(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if t.contains(['+', '*', '/', '(', ')']) {
        return true;
    }
    // Binary minus (not a lone leading sign on a simple number).
    if t.chars().skip(1).any(|c| c == '-') {
        return true;
    }
    has_length_unit_suffix(t)
}

/// Format a length in millimetres for display above an expression field.
pub fn format_length_display(v: f32) -> String {
    if v.abs() < 0.1 {
        "0".to_string()
    } else {
        format!("{:.1}", v)
    }
}

/// Parse a length expression, falling back when empty/invalid.
pub fn parse_length_or(text: &str, fallback: f32) -> f32 {
    eval_length_mm(text).unwrap_or(fallback)
}

/// Parse a positive length expression, falling back when empty/invalid/non-positive.
pub fn parse_positive_length_or(text: &str, fallback: f32) -> f32 {
    eval_length_mm(text)
        .filter(|v| *v > 0.0)
        .unwrap_or(fallback)
}

fn has_length_unit_suffix(text: &str) -> bool {
    const UNITS: &[&str] = &["mm", "cm", "ft", "in", "m"];
    let lower: String = text
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect();
    UNITS.iter().any(|unit| {
        lower.ends_with(unit)
            && lower
                .strip_suffix(unit)
                .is_some_and(|prefix| prefix.chars().last().is_some_and(|c| c.is_ascii_digit()))
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LengthUnit {
    Mm,
    Cm,
    M,
    Ft,
    In,
}

impl LengthUnit {
    fn to_mm(self) -> f32 {
        match self {
            LengthUnit::Mm => 1.0,
            LengthUnit::Cm => 10.0,
            LengthUnit::M => 1000.0,
            LengthUnit::Ft => 304.8,
            LengthUnit::In => 25.4,
        }
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.chars.peek().is_none()
    }

    fn skip_ws(&mut self) {
        while matches!(self.chars.peek(), Some(' ' | '\t')) {
            self.chars.next();
        }
    }

    fn bump(&mut self) -> Option<char> {
        self.chars.next()
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn parse_expr(&mut self) -> Result<f32, ()> {
        self.parse_add_sub()
    }

    fn parse_add_sub(&mut self) -> Result<f32, ()> {
        let mut acc = self.parse_mul_div()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.bump();
                    acc += self.parse_mul_div()?;
                }
                Some('-') => {
                    self.bump();
                    acc -= self.parse_mul_div()?;
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    fn parse_mul_div(&mut self) -> Result<f32, ()> {
        let mut acc = self.parse_unary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.bump();
                    acc *= self.parse_unary()?;
                }
                Some('/') => {
                    self.bump();
                    let rhs = self.parse_unary()?;
                    if rhs.abs() < f32::EPSILON {
                        return Err(());
                    }
                    acc /= rhs;
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    fn parse_unary(&mut self) -> Result<f32, ()> {
        self.skip_ws();
        match self.peek() {
            Some('-') => {
                self.bump();
                Ok(-self.parse_unary()?)
            }
            Some('+') => {
                self.bump();
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f32, ()> {
        self.skip_ws();
        if self.peek() == Some('(') {
            self.bump();
            let v = self.parse_expr()?;
            self.skip_ws();
            if self.peek() != Some(')') {
                return Err(());
            }
            self.bump();
            return Ok(v);
        }
        self.parse_quantity()
    }

    fn parse_quantity(&mut self) -> Result<f32, ()> {
        self.skip_ws();
        let n = self.parse_number()?;
        let unit = self.parse_unit()?;
        Ok(n * unit.to_mm())
    }

    fn parse_number(&mut self) -> Result<f32, ()> {
        self.skip_ws();
        let mut s = String::new();
        let mut saw_digit = false;
        let mut saw_dot = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                saw_digit = true;
                s.push(c);
                self.bump();
            } else if c == '.' && !saw_dot {
                saw_dot = true;
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if !saw_digit {
            return Err(());
        }
        s.parse::<f32>().map_err(|_| ())
    }

    fn parse_unit(&mut self) -> Result<LengthUnit, ()> {
        self.skip_ws();
        let rest: String = self.chars.clone().collect();
        let lower: String = rest
            .chars()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        for (suffix, unit, len) in [
            ("mm", LengthUnit::Mm, 2),
            ("cm", LengthUnit::Cm, 2),
            ("ft", LengthUnit::Ft, 2),
            ("in", LengthUnit::In, 2),
            ("m", LengthUnit::M, 1),
        ] {
            if lower.starts_with(suffix) {
                let next = lower.as_bytes().get(len).copied();
                if next.is_none_or(|b| !b.is_ascii_alphabetic()) {
                    for _ in 0..len {
                        self.bump();
                    }
                    return Ok(unit);
                }
            }
        }
        Ok(LengthUnit::Mm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_number_is_mm() {
        assert!((eval_length_mm("10").unwrap() - 10.0).abs() < 1e-4);
        assert!((eval_length_mm("  3.5  ").unwrap() - 3.5).abs() < 1e-4);
    }

    #[test]
    fn unit_conversions() {
        assert!((eval_length_mm("1cm").unwrap() - 10.0).abs() < 1e-4);
        assert!((eval_length_mm("1m").unwrap() - 1000.0).abs() < 1e-4);
        assert!((eval_length_mm("1ft").unwrap() - 304.8).abs() < 1e-4);
        assert!((eval_length_mm("1in").unwrap() - 25.4).abs() < 1e-4);
        assert!((eval_length_mm("2 in").unwrap() - 50.8).abs() < 1e-4);
    }

    #[test]
    fn mixed_expression() {
        let v = eval_length_mm("2in + 5mm / 2").unwrap();
        assert!((v - 53.3).abs() < 1e-3, "got {v}");
    }

    #[test]
    fn arithmetic_precedence() {
        assert!((eval_length_mm("2 + 3 * 4").unwrap() - 14.0).abs() < 1e-4);
        assert!((eval_length_mm("(2 + 3) * 4").unwrap() - 20.0).abs() < 1e-4);
    }

    #[test]
    fn signed_lengths() {
        assert!((eval_length_mm("-5mm").unwrap() + 5.0).abs() < 1e-4);
        assert!((eval_length_mm("10mm - 15mm").unwrap() + 5.0).abs() < 1e-4);
    }

    #[test]
    fn invalid_expressions_return_none() {
        assert!(eval_length_mm("").is_none());
        assert!(eval_length_mm("abc").is_none());
        assert!(eval_length_mm("12x").is_none());
        assert!(eval_length_mm("2in +").is_none());
    }

    #[test]
    fn shows_computed_length_detects_syntax() {
        assert!(!shows_computed_length(""));
        assert!(!shows_computed_length("50"));
        assert!(!shows_computed_length("50.0"));
        assert!(shows_computed_length("2in"));
        assert!(shows_computed_length("2in + 5mm / 2"));
        assert!(shows_computed_length("(10 + 5)mm"));
        assert!(shows_computed_length("10 - 5"));
    }

    #[test]
    fn parse_positive_length_or_rejects_non_positive() {
        assert!((parse_positive_length_or("0", 9.0) - 9.0).abs() < 1e-4);
        assert!((parse_positive_length_or("-3", 9.0) - 9.0).abs() < 1e-4);
        assert!((parse_positive_length_or("2in", 9.0) - 50.8).abs() < 1e-3);
    }

    #[test]
    fn expression_string_round_trips_via_eval() {
        let expr = "2in + 5mm / 2";
        let v = eval_length_mm(expr).unwrap();
        assert!((v - 53.3).abs() < 1e-3);
        // Stored text is preserved by callers; re-evaluating yields the same value.
        assert!((eval_length_mm(expr).unwrap() - v).abs() < 1e-6);
    }
}