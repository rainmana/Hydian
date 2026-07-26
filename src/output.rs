use std::io::{self, Write};

use anyhow::Error;
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::Cli;

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Printer {
    json: bool,
    plain: bool,
    quiet: bool,
}

impl Printer {
    #[must_use]
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            json: cli.json,
            plain: cli.plain || cli.no_color,
            quiet: cli.quiet,
        }
    }

    #[must_use]
    pub const fn is_json(&self) -> bool {
        self.json
    }

    pub fn success<T: Serialize>(&self, command: &str, data: &T, human: &[String]) {
        if self.json {
            write_json(&json!({
                "schema_version": OUTPUT_SCHEMA_VERSION,
                "command": command,
                "ok": true,
                "data": data,
                "error": Value::Null,
            }));
        } else if !self.quiet {
            let mut stdout = io::stdout().lock();
            for line in human {
                let _ = writeln!(stdout, "{line}");
            }
        }
    }

    pub fn failure(&self, command: &str, error: &Error, debug: bool) {
        if self.json {
            write_json_to(
                io::stderr().lock(),
                &json!({
                    "schema_version": OUTPUT_SCHEMA_VERSION,
                    "command": command,
                    "ok": false,
                    "data": Value::Null,
                    "error": {
                        "code": "operation_failed",
                        "message": error.to_string(),
                    },
                }),
            );
            return;
        }

        let symbol = if self.plain { "FAILED" } else { "✗ FAILED" };
        eprintln!("{symbol}: {error}");
        if debug {
            eprintln!();
            eprintln!("DETAILS:");
            eprintln!("    {error:?}");
        }
    }
}

fn write_json(value: &Value) {
    write_json_to(io::stdout().lock(), value);
}

fn write_json_to(mut writer: impl Write, value: &Value) {
    if serde_json::to_writer_pretty(&mut writer, value).is_ok() {
        let _ = writeln!(writer);
    }
}

#[cfg(test)]
mod tests {
    use super::OUTPUT_SCHEMA_VERSION;

    #[test]
    fn machine_output_schema_is_explicitly_versioned() {
        assert_eq!(OUTPUT_SCHEMA_VERSION, 1);
    }
}
