// From https://github.com/LucentFlux/wgsl-minifier/

use std::borrow::Cow;
use std::collections::HashMap;

fn is_numeric(c: char) -> bool {
    c.is_ascii_digit()
}

/// Removes all the characters it can in some wgsl sourcecode without joining any keywords or identifiers together.
pub fn minify_wgsl_source(src: &str) -> String {
    let mut src = Cow::<'_, str>::Borrowed(src);

    // Remove whitespace
    let mut new_src = String::new();
    let mut last_char = ' ';
    let mut chars = src.chars().peekable();
    while let Some(current_char) = chars.next() {
        let next_char = *chars.peek().unwrap_or(&' ');

        if current_char.is_whitespace() {
            // Only keep whitespace if it separates identifiers,
            // or separates a hyphen from a literal (since older versions of the spec require whitespace)
            if (unicode_ident::is_xid_continue(last_char)
                && unicode_ident::is_xid_continue(next_char))
                || (last_char == '-' && (is_numeric(next_char) || next_char == '.'))
            {
                new_src.push(' ');
                last_char = ' ';
            }
            continue;
        }

        new_src.push(current_char);
        last_char = current_char;
    }
    src = Cow::Owned(new_src);

    // Anything of the form `,}` or `,)` or `,]` can have the comma removed
    new_src = String::new();
    chars = src.chars().peekable();
    while let Some(current_char) = chars.next() {
        let next_char = *chars.peek().unwrap_or(&' ');

        if current_char == ',' && matches!(next_char, '}' | ')' | ']') {
            continue;
        }

        new_src.push(current_char);
    }
    src = Cow::Owned(new_src);

    // Get rid of double parentheses
    let mut parentheses = HashMap::new(); // Map from parenthesis starts to ends
    let mut unclosed_stack = Vec::new();
    for (i, c) in src.chars().enumerate() {
        if c == '(' {
            unclosed_stack.push(i);
        } else if c == ')' {
            let start = unclosed_stack.pop().expect("wgsl parentheses are balanced");
            parentheses.insert(start, i);
        }
    }
    assert!(unclosed_stack.is_empty());
    new_src = String::new();
    let mut to_drop_stack = Vec::new();
    for (i, c) in src.chars().enumerate() {
        if let Some(outer_end) = parentheses.get(&i) {
            if let Some(inner_end) = parentheses.get(&(i + 1)) {
                if *outer_end == *inner_end + 1 {
                    to_drop_stack.push(*outer_end);
                    continue;
                }
            }
        }
        if let Some(next_to_skip) = to_drop_stack.last() {
            if *next_to_skip == i {
                to_drop_stack.pop();
                continue;
            }
        }
        new_src.push(c);
    }
    src = Cow::Owned(new_src);

    src.to_string()
}