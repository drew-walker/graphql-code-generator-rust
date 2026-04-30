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

/// Mirrors upstream `transformComment` (`packages/plugins/other/visitor-plugin-common/src/utils.ts`).
pub fn transform_comment(description: &str, indent_level: usize, disabled: bool) -> String {
    if disabled || description.is_empty() {
        return String::new();
    }

    let comment = description.replace("*/", "*\\/");
    let lines: Vec<&str> = comment.split('\n').collect();
    let indent = "  ".repeat(indent_level);

    if lines.len() == 1 {
        return format!("{indent}/** {} */\n", strip_trailing_spaces(lines[0]));
    }

    // Mirrors upstream:
    // lines = ['/**', ...lines.map(line => ` * ${line}`), ' */\n'];
    // return stripTrailingSpaces(lines.map(line => indent(line, indentLevel)).join('\n'));
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len() + 2);
    out_lines.push("/**".to_string());
    for line in lines {
        out_lines.push(format!(" * {}", strip_trailing_spaces(line)));
    }
    out_lines.push(" */\n".to_string());

    let joined = out_lines
        .into_iter()
        .map(|l| format!("{indent}{l}"))
        .collect::<Vec<_>>()
        .join("\n");
    strip_trailing_spaces_multiline(joined)
}

fn strip_trailing_spaces(s: &str) -> &str {
    s.trim_end_matches([' ', '\t', '\r'])
}

fn strip_trailing_spaces_multiline(s: String) -> String {
    // Like upstream `stripTrailingSpaces`: trim trailing whitespace on each line, preserve newlines.
    // We operate on '\n' boundaries; the final output should keep the same newline structure.
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(strip_trailing_spaces(line));
    }
    out
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
