//! Mechanical port of `packages/plugins/other/visitor-plugin-common/src/utils.ts`.

/// Port of TS `wrapWithSingleQuotes(value: string | number | NameNode, skipNumericCheck = false)`.
///
/// `NameNode` is not modeled yet; call sites that need it can pass the identifier as `&str`.
pub fn wrap_with_single_quotes(value: WrapInput<'_>, skip_numeric_check: bool) -> String {
    match value {
        WrapInput::Number(n) => f64_to_js_string(n),
        WrapInput::Str(s) => wrap_string(s, skip_numeric_check),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WrapInput<'a> {
    Str(&'a str),
    Number(f64),
}

fn wrap_string(value: &str, skip_numeric_check: bool) -> String {
    if skip_numeric_check {
        return format!("'{value}'");
    }

    if is_js_numeric_string(value) {
        return value.to_string();
    }

    format!("'{value}'")
}

/// Mirrors TS:
/// `(typeof value === 'string' && !Number.isNaN(parseInt(value)) && parseFloat(value).toString() === value)`
fn is_js_numeric_string(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if js_parse_int(value).is_none() {
        return false;
    }
    let Some(pf) = js_parse_float(value) else {
        return false;
    };
    f64_to_js_string(pf) == value
}

/// Subset of ECMAScript `parseInt(string, 10)` (leading whitespace, optional sign, digit run).
fn js_parse_int(value: &str) -> Option<i64> {
    let s = value.trim_start();
    if s.is_empty() {
        return None;
    }
    let mut chars = s.chars().peekable();
    let mut sign = 1i64;
    match chars.peek() {
        Some('+') => {
            chars.next();
        }
        Some('-') => {
            sign = -1;
            chars.next();
        }
        _ => {}
    }
    let mut acc: i64 = 0;
    let mut any = false;
    for c in chars {
        if c.is_ascii_digit() {
            any = true;
            acc = acc
                .saturating_mul(10)
                .saturating_add((c as u8 - b'0') as i64);
        } else {
            break;
        }
    }
    any.then_some(acc.saturating_mul(sign))
}

/// Subset of ECMAScript `parseFloat(string)` sufficient for codegen string literals.
fn js_parse_float(value: &str) -> Option<f64> {
    let s = value.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok()
}

/// `String(number)` / `Number.prototype.toString`-ish for stable enum literals.
fn f64_to_js_string(n: f64) -> String {
    if n == 0.0 && n.is_sign_negative() {
        return "-0".to_string();
    }
    if n.is_finite() && n.fract() == 0.0 && n.abs() <= (i64::MAX as f64) {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qux_string_quoted() {
        assert_eq!(
            wrap_with_single_quotes(WrapInput::Str("QUX"), true),
            "'QUX'"
        );
    }

    #[test]
    fn numeric_string_unquoted_when_not_skip() {
        assert_eq!(wrap_with_single_quotes(WrapInput::Str("10"), false), "10");
    }

    #[test]
    fn number_unquoted() {
        assert_eq!(wrap_with_single_quotes(WrapInput::Number(42.0), true), "42");
    }
}
