//! Turns the user's notification command template into an argument list.
//!
//! The template is split like a shell would (quotes and backslashes) but is never run through
//! one, so text substituted for `{title}` and `{message}` always stays a single argument part.

/// Command used until the user changes it.
pub const DEFAULT_NOTIFY_COMMAND: &str = "notify-send --app-name=Snapforge {title} {message}";

/// Splits `template` into words. Single quotes keep everything literal; double quotes allow
/// `\"` and `\\`; outside quotes a backslash escapes the next character.
pub fn split_command(template: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("unterminated ' quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => cur.push(e),
                            Some(e) => {
                                cur.push('\\');
                                cur.push(e);
                            }
                            None => return Err("unterminated \" quote".into()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err("unterminated \" quote".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(e) => cur.push(e),
                    None => return Err("trailing backslash".into()),
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

/// Splits `template` and substitutes `{title}` and `{message}` inside each word.
/// Fails when the template is empty or has an unbalanced quote.
pub fn parse_command(template: &str, title: &str, message: &str) -> Result<Vec<String>, String> {
    let words = split_command(template)?;
    if words.is_empty() {
        return Err("the command is empty".into());
    }
    Ok(words
        .into_iter()
        .map(|w| w.replace("{title}", title).replace("{message}", message))
        .collect())
}

/// Whether `template` can be turned into a command at all.
pub fn check_command(template: &str) -> Result<(), String> {
    parse_command(template, "", "").map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_command_fills_in_the_title_and_message() {
        let argv = parse_command(
            DEFAULT_NOTIFY_COMMAND,
            "Snapforge",
            "Copied to the clipboard",
        )
        .unwrap();
        assert_eq!(
            argv,
            [
                "notify-send",
                "--app-name=Snapforge",
                "Snapforge",
                "Copied to the clipboard"
            ]
        );
    }

    #[test]
    fn quotes_group_words_and_placeholders_work_inside_them() {
        let argv = parse_command("say \"{title}: {message}\" 'a b'", "T", "M").unwrap();
        assert_eq!(argv, ["say", "T: M", "a b"]);
    }

    #[test]
    fn a_message_with_shell_characters_stays_one_argument() {
        let argv = parse_command("echo {message}", "t", "a; rm -rf ~ && $(x) `y` 'z'").unwrap();
        assert_eq!(argv, ["echo", "a; rm -rf ~ && $(x) `y` 'z'"]);
    }

    #[test]
    fn backslashes_escape_spaces_and_quotes() {
        assert_eq!(split_command(r#"a\ b \"c"#).unwrap(), ["a b", "\"c"]);
        assert_eq!(split_command(r#""a \" b""#).unwrap(), ["a \" b"]);
    }

    #[test]
    fn an_empty_quoted_word_is_kept() {
        assert_eq!(split_command("a '' b").unwrap(), ["a", "", "b"]);
    }

    #[test]
    fn an_empty_template_is_an_error() {
        assert!(parse_command("   ", "t", "m").is_err());
        assert!(check_command("").is_err());
    }

    #[test]
    fn an_unbalanced_quote_is_an_error() {
        assert!(split_command("a 'b").is_err());
        assert!(split_command("a \"b").is_err());
        assert!(split_command("a\\").is_err());
    }

    #[test]
    fn the_default_command_is_valid() {
        assert!(check_command(DEFAULT_NOTIFY_COMMAND).is_ok());
    }
}
