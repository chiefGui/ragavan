use anstyle::{AnsiColor, Effects, Style};
use clap::builder::styling::Styles;
use ragavan_diagnostics::{Diagnostic, Value};
use serde_json::{Map, Value as JsonValue, json};
use std::{fmt, io, io::Write as _};

const ERROR_STYLE: Style = AnsiColor::Red.on_default().effects(Effects::BOLD);
const SUCCESS_STYLE: Style = AnsiColor::Green.on_default().effects(Effects::BOLD);
const CODE_STYLE: Style = AnsiColor::Cyan.on_default();
const LABEL_STYLE: Style = AnsiColor::Cyan.on_default().effects(Effects::BOLD);
const HELP_STYLE: Style = LABEL_STYLE;

pub(super) const CLI_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Green.on_default())
    .placeholder(AnsiColor::Cyan.on_default());

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Format {
    Human,
    Json,
}

pub(super) trait Response {
    fn write_human(&self, output: &mut HumanOutput<'_>) -> io::Result<()>;

    fn json_object(&self) -> Map<String, JsonValue>;
}

pub(super) struct HumanOutput<'a> {
    writer: &'a mut dyn io::Write,
}

impl<'a> HumanOutput<'a> {
    fn new(writer: &'a mut dyn io::Write) -> Self {
        Self { writer }
    }

    pub(super) fn success(&mut self, message: fmt::Arguments<'_>) -> io::Result<()> {
        writeln!(
            self.writer,
            "{SUCCESS_STYLE}{message}{reset}",
            reset = SUCCESS_STYLE.render_reset()
        )
    }

    pub(super) fn line(&mut self, message: fmt::Arguments<'_>) -> io::Result<()> {
        writeln!(self.writer, "{message}")
    }

    pub(super) fn field(&mut self, label: &str, value: fmt::Arguments<'_>) -> io::Result<()> {
        writeln!(
            self.writer,
            "{LABEL_STYLE}{label}{reset}: {value}",
            reset = LABEL_STYLE.render_reset()
        )
    }

    pub(super) fn item(&mut self, value: fmt::Arguments<'_>) -> io::Result<()> {
        writeln!(self.writer, "  {value}")
    }
}

pub(super) fn present(response: &impl Response, format: Format) -> io::Result<()> {
    let result = match format {
        Format::Human => {
            let mut writer = anstream::stdout().lock();
            response.write_human(&mut HumanOutput::new(&mut writer))
        }
        Format::Json => writeln!(
            anstream::stdout().lock(),
            "{}",
            JsonValue::Object(response.json_object())
        ),
    };
    ignore_broken_pipe(result)
}

pub(super) fn print(value: impl fmt::Display) -> io::Result<()> {
    ignore_broken_pipe(write!(anstream::stdout().lock(), "{value}"))
}

pub(super) fn report(error: &dyn Diagnostic, format: Format, exit_code: i32) -> i32 {
    let result = match format {
        Format::Human => write_human_diagnostic(&mut anstream::stderr().lock(), error),
        Format::Json => writeln!(anstream::stderr().lock(), "{}", diagnostic_json(error)),
    };
    if result.is_ok() { exit_code } else { 1 }
}

fn write_human_diagnostic(writer: &mut impl io::Write, error: &dyn Diagnostic) -> io::Result<()> {
    writeln!(
        writer,
        "{ERROR_STYLE}error{error_reset}{CODE_STYLE}[{}]{code_reset}: {error}",
        error.code(),
        error_reset = ERROR_STYLE.render_reset(),
        code_reset = CODE_STYLE.render_reset(),
    )?;
    if let Some(help) = error.help() {
        writeln!(
            writer,
            "\n  {HELP_STYLE}help{reset}: {help}",
            reset = HELP_STYLE.render_reset()
        )?;
    }
    Ok(())
}

fn diagnostic_json(error: &dyn Diagnostic) -> JsonValue {
    let details = error
        .details()
        .into_iter()
        .map(|detail| {
            let value = match detail.value() {
                Value::Text(value) => JsonValue::String(value.clone()),
                Value::Number(value) => JsonValue::from(*value),
                Value::List(values) => {
                    JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect())
                }
            };
            (detail.name().to_owned(), value)
        })
        .collect::<Map<_, _>>();
    let root_message = error.to_string();
    let mut previous = root_message.clone();
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(source_error) = source {
        let cause = source_error.to_string();
        if cause != previous {
            previous.clone_from(&cause);
            causes.push(cause);
        }
        source = source_error.source();
    }

    json!({
        "error": {
            "code": error.code(),
            "message": root_message,
            "help": error.help(),
            "details": details,
            "causes": causes,
        },
    })
}

fn ignore_broken_pipe(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}
