//! A fail-closed parser for static package scripts with `&&` sequencing.
//!
//! Runner adapters opt into this common subset only when they can deliver
//! trailing arguments to one invocation. Other shell control flow remains
//! unsupported until its argument-delivery semantics can be proven.

use std::{fmt, iter::Peekable, path::Path, str::CharIndices};

pub(super) struct Script {
    invocations: Vec<Invocation>,
}

impl Script {
    pub(super) fn parse(source: &str) -> Result<Self, Error> {
        Parser::new(source).parse()
    }

    pub(super) fn invocations(&self) -> &[Invocation] {
        &self.invocations
    }
}

pub(super) struct Invocation {
    program: String,
    arguments: Vec<String>,
}

impl Invocation {
    pub(super) fn invokes(&self, expected: &str) -> bool {
        let Some(program) = Path::new(&self.program)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return false;
        };
        if program == expected {
            return true;
        }
        if !cfg!(windows) {
            return false;
        }

        program.eq_ignore_ascii_case(expected)
            || program.rsplit_once('.').is_some_and(|(stem, extension)| {
                stem.eq_ignore_ascii_case(expected)
                    && (extension.eq_ignore_ascii_case("cmd")
                        || extension.eq_ignore_ascii_case("exe"))
            })
    }

    pub(super) fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

struct Parser<'a> {
    characters: Peekable<CharIndices<'a>>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            characters: source.char_indices().peekable(),
        }
    }

    fn parse(mut self) -> Result<Script, Error> {
        let mut invocations = Vec::new();

        loop {
            self.skip_horizontal_whitespace();
            let Some((position, _)) = self.characters.peek().copied() else {
                if invocations.is_empty() {
                    return Err(Error::new(0, ErrorKind::EmptyScript));
                }
                break;
            };

            let words = self.parse_command()?;
            invocations.push(Invocation::from_words(words, position)?);
            self.skip_horizontal_whitespace();

            let Some((position, character)) = self.characters.next() else {
                break;
            };
            if character != '&' || self.characters.next_if(|(_, next)| *next == '&').is_none() {
                return Err(Error::new(
                    position,
                    ErrorKind::UnsupportedOperator(character),
                ));
            }

            self.skip_horizontal_whitespace();
            if self.characters.peek().is_none() {
                return Err(Error::new(position, ErrorKind::MissingCommandAfterAnd));
            }
        }

        Ok(Script { invocations })
    }

    fn parse_command(&mut self) -> Result<Vec<Word>, Error> {
        let mut words = Vec::new();

        loop {
            self.skip_horizontal_whitespace();
            let Some((position, character)) = self.characters.peek().copied() else {
                break;
            };
            if character == '&' {
                break;
            }
            if character == '\r' || character == '\n' {
                return Err(Error::new(position, ErrorKind::UnsupportedNewline));
            }
            if is_unsupported_operator(character) {
                return Err(Error::new(
                    position,
                    ErrorKind::UnsupportedOperator(character),
                ));
            }
            if character == '#' {
                return Err(Error::new(position, ErrorKind::UnsupportedComment));
            }

            words.push(self.parse_word()?);
        }

        if words.is_empty() {
            let position = self.characters.peek().map_or(0, |(position, _)| *position);
            Err(Error::new(position, ErrorKind::MissingCommand))
        } else {
            Ok(words)
        }
    }

    fn parse_word(&mut self) -> Result<Word, Error> {
        let mut word = Word::new();

        while let Some((position, character)) = self.characters.peek().copied() {
            match character {
                ' ' | '\t' => break,
                '\r' | '\n' => {
                    return Err(Error::new(position, ErrorKind::UnsupportedNewline));
                }
                '&' => break,
                character if is_unsupported_operator(character) => {
                    return Err(Error::new(
                        position,
                        ErrorKind::UnsupportedOperator(character),
                    ));
                }
                '$' | '`' => {
                    return Err(Error::new(position, ErrorKind::UnsupportedExpansion));
                }
                '\'' => self.parse_single_quoted(&mut word, position)?,
                '"' => self.parse_double_quoted(&mut word, position)?,
                '\\' => self.parse_escape(&mut word, position)?,
                _ => {
                    self.characters.next();
                    word.push_unquoted(character);
                }
            }
        }

        Ok(word)
    }

    fn parse_single_quoted(&mut self, word: &mut Word, start: usize) -> Result<(), Error> {
        self.characters.next();
        word.mark_quoted();

        for (_, character) in self.characters.by_ref() {
            if character == '\'' {
                return Ok(());
            }
            word.value.push(character);
        }

        Err(Error::new(start, ErrorKind::UnclosedSingleQuote))
    }

    fn parse_double_quoted(&mut self, word: &mut Word, start: usize) -> Result<(), Error> {
        self.characters.next();
        word.mark_quoted();

        while let Some((position, character)) = self.characters.next() {
            match character {
                '"' => return Ok(()),
                '$' | '`' => {
                    return Err(Error::new(position, ErrorKind::UnsupportedExpansion));
                }
                '\r' | '\n' => {
                    return Err(Error::new(position, ErrorKind::UnsupportedNewline));
                }
                '\\' => {
                    let Some((escaped_position, escaped)) = self.characters.next() else {
                        return Err(Error::new(position, ErrorKind::TrailingEscape));
                    };
                    match escaped {
                        '"' | '\\' | '$' | '`' => word.value.push(escaped),
                        '\r' | '\n' => {
                            return Err(Error::new(
                                escaped_position,
                                ErrorKind::UnsupportedNewline,
                            ));
                        }
                        _ => {
                            word.value.push('\\');
                            word.value.push(escaped);
                        }
                    }
                }
                _ => word.value.push(character),
            }
        }

        Err(Error::new(start, ErrorKind::UnclosedDoubleQuote))
    }

    fn parse_escape(&mut self, word: &mut Word, position: usize) -> Result<(), Error> {
        self.characters.next();
        word.mark_quoted();
        let Some((escaped_position, escaped)) = self.characters.next() else {
            return Err(Error::new(position, ErrorKind::TrailingEscape));
        };
        if escaped == '\r' || escaped == '\n' {
            return Err(Error::new(escaped_position, ErrorKind::UnsupportedNewline));
        }
        word.value.push(escaped);
        Ok(())
    }

    fn skip_horizontal_whitespace(&mut self) {
        while self
            .characters
            .next_if(|(_, character)| matches!(character, ' ' | '\t'))
            .is_some()
        {}
    }
}

impl Invocation {
    fn from_words(words: Vec<Word>, position: usize) -> Result<Self, Error> {
        let mut words = words.into_iter().skip_while(Word::is_assignment);
        let Some(program) = words.next() else {
            return Err(Error::new(position, ErrorKind::MissingExecutable));
        };
        if program.value.is_empty() {
            return Err(Error::new(position, ErrorKind::MissingExecutable));
        }

        Ok(Self {
            program: program.value,
            arguments: words.map(|word| word.value).collect(),
        })
    }
}

struct Word {
    value: String,
    assignment: bool,
    assignment_name_possible: bool,
}

impl Word {
    fn new() -> Self {
        Self {
            value: String::new(),
            assignment: false,
            assignment_name_possible: true,
        }
    }

    fn push_unquoted(&mut self, character: char) {
        if character == '=' && self.assignment_name_possible {
            self.assignment = !self.value.is_empty();
            self.assignment_name_possible = false;
        } else if !self.assignment {
            self.assignment_name_possible &= is_name_character(character, self.value.is_empty());
        }
        self.value.push(character);
    }

    fn mark_quoted(&mut self) {
        if !self.assignment {
            self.assignment_name_possible = false;
        }
    }

    fn is_assignment(&self) -> bool {
        self.assignment
    }
}

fn is_name_character(character: char, first: bool) -> bool {
    character == '_' || character.is_ascii_alphabetic() || (!first && character.is_ascii_digit())
}

fn is_unsupported_operator(character: char) -> bool {
    matches!(character, '|' | ';' | '<' | '>' | '(' | ')' | '{' | '}')
}

#[derive(Debug)]
pub(super) struct Error {
    position: usize,
    kind: ErrorKind,
}

impl Error {
    fn new(position: usize, kind: ErrorKind) -> Self {
        Self { position, kind }
    }
}

#[derive(Debug)]
enum ErrorKind {
    EmptyScript,
    MissingCommand,
    MissingCommandAfterAnd,
    MissingExecutable,
    TrailingEscape,
    UnclosedSingleQuote,
    UnclosedDoubleQuote,
    UnsupportedComment,
    UnsupportedExpansion,
    UnsupportedNewline,
    UnsupportedOperator(char),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "at byte {}: ", self.position)?;
        match self.kind {
            ErrorKind::EmptyScript => formatter.write_str("the script is empty"),
            ErrorKind::MissingCommand => formatter.write_str("a command is missing"),
            ErrorKind::MissingCommandAfterAnd => {
                formatter.write_str("a command is required after `&&`")
            }
            ErrorKind::MissingExecutable => formatter.write_str("a command contains no executable"),
            ErrorKind::TrailingEscape => formatter.write_str("a trailing escape is incomplete"),
            ErrorKind::UnclosedSingleQuote => formatter.write_str("a single quote is not closed"),
            ErrorKind::UnclosedDoubleQuote => formatter.write_str("a double quote is not closed"),
            ErrorKind::UnsupportedComment => {
                formatter.write_str("shell comments are not supported")
            }
            ErrorKind::UnsupportedExpansion => {
                formatter.write_str("dynamic shell expansion is not supported")
            }
            ErrorKind::UnsupportedNewline => {
                formatter.write_str("multi-line shell scripts are not supported")
            }
            ErrorKind::UnsupportedOperator(operator) => {
                write!(formatter, "shell operator `{operator}` is not supported")
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::Script;

    #[test]
    fn parses_static_and_chains_without_inspecting_command_names() {
        let script = Script::parse(
            r#"MODE="development mode" build-tool run compile && "./bin/server" dev"#,
        )
        .expect("a static setup chain should parse");
        let invocations = script.invocations();

        assert_eq!(invocations.len(), 2);
        assert!(invocations[0].invokes("build-tool"));
        assert_eq!(invocations[0].arguments(), ["run", "compile"]);
        assert!(invocations[1].invokes("server"));
        assert_eq!(invocations[1].arguments(), ["dev"]);
    }

    #[test]
    fn keeps_quoted_operators_inside_arguments() {
        let script = Script::parse(r#"echo "setup && ready" && server dev"#)
            .expect("quoted operators should be ordinary text");
        let invocations = script.invocations();

        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].arguments(), ["setup && ready"]);
    }

    #[test]
    fn executable_matching_uses_platform_path_rules() {
        let script =
            Script::parse(r#""tools/SERVER.CMD""#).expect("a quoted executable path should parse");
        assert_eq!(script.invocations()[0].invokes("server"), cfg!(windows));

        let script =
            Script::parse(r#""tools\server""#).expect("a quoted executable path should parse");
        assert_eq!(script.invocations()[0].invokes("server"), cfg!(windows));
    }

    #[test]
    fn rejects_graphs_that_do_not_have_a_single_argument_sink() {
        for source in [
            "server | worker",
            "server || worker",
            "server & worker",
            "server; worker",
            "server > output.log",
            "(server)",
            "server\nworker",
        ] {
            assert!(Script::parse(source).is_err(), "`{source}` should fail");
        }
    }

    #[test]
    fn rejects_dynamic_and_comment_syntax() {
        for source in ["$SERVER", "$(server)", "`server`", "server # comment"] {
            assert!(Script::parse(source).is_err(), "`{source}` should fail");
        }
    }

    #[test]
    fn rejects_incomplete_static_syntax() {
        for source in [
            "",
            "   ",
            "&& server",
            "server &&",
            "'server",
            "\"server",
            "server\\",
            "MODE=development",
        ] {
            assert!(Script::parse(source).is_err(), "`{source}` should fail");
        }
    }
}
