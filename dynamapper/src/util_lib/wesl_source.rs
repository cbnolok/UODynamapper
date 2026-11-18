//! Reading a shader's source in tests: the value of a constant, the body of a function. Tests that guard a rule
//! whose numbers live in a WESL file read them here rather than repeat them, so the guard follows the shader.

/// The value of `const <name>: <type> = <value>;` in the WESL `source`, as written. Panics when `source` declares no
/// such constant.
pub fn wesl_const<'a>(source: &'a str, name: &str) -> &'a str {
    let declaration = format!("const {name}:");
    let start = source
        .lines()
        .find(|line| line.trim_start().starts_with(&declaration))
        .unwrap_or_else(|| panic!("no `const {name}` in the shader source"));
    let (_, value) = start.split_once('=').unwrap_or_else(|| panic!("`const {name}` has no value"));
    value.trim().trim_end_matches(';').trim()
}

/// `wesl_const` of an `f32` constant written as a plain number.
pub fn wesl_f32_const(source: &str, name: &str) -> f32 {
    let value = wesl_const(source, name);
    value.parse().unwrap_or_else(|_| panic!("`const {name}` is `{value}`, not a plain f32"))
}

/// The body of `fn <name>(` in the WESL `source`, from its opening brace to the matching closing one. Panics when
/// `source` defines no such function.
pub fn wesl_fn_body<'a>(source: &'a str, name: &str) -> &'a str {
    let head = format!("fn {name}(");
    let start = source.find(&head).unwrap_or_else(|| panic!("no `fn {name}` in the shader source"));
    let open = start + source[start..].find('{').unwrap_or_else(|| panic!("`fn {name}` has no body"));
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("`fn {name}` has no closing brace")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "// A note.\nconst A_SCALE: f32 = 0.25;\n  const B_FLAG: u32 = 1u << 3u;\n\
        fn outer(x: f32) -> f32 {\n  if (x > 0.0) { return x; }\n  return -x;\n}\nfn after() {}\n";

    #[test]
    fn reads_constants_and_function_bodies() {
        assert_eq!(wesl_f32_const(SOURCE, "A_SCALE"), 0.25);
        assert_eq!(wesl_const(SOURCE, "B_FLAG"), "1u << 3u");
        assert_eq!(wesl_fn_body(SOURCE, "outer"), "{\n  if (x > 0.0) { return x; }\n  return -x;\n}");
    }
}
