//! Demonstrates live parser.

use std::io::{self, IsTerminal, Read, Write};
use std::process::Command;

use m2_syn::{
    LexError, NativeRealtimeParser, Spanned, TextEdit, format_pretty_cell_tokens,
    format_pretty_tree,
};

/// State used by the TerminalMode example.
struct TerminalMode(Option<String>);

impl TerminalMode {
    /// Handles raw.
    fn raw() -> Self {
        if !io::stdin().is_terminal() {
            return Self(None);
        }
        let Ok(saved) = Command::new("stty").arg("-g").output() else {
            return Self(None);
        };
        if !saved.status.success() {
            return Self(None);
        }
        let saved = String::from_utf8_lossy(&saved.stdout).trim().to_owned();
        let enabled = Command::new("stty")
            .args(["raw", "-echo"])
            .status()
            .is_ok_and(|status| status.success());
        Self(enabled.then_some(saved))
    }
}

impl Drop for TerminalMode {
    fn drop(&mut self) {
        if let Some(saved) = &self.0 {
            let _ = Command::new("stty").arg(saved).status();
        }
    }
}

/// Handles write diagnostic.
fn write_diagnostic(output: &mut String, diagnostic: &LexError) {
    let span = diagnostic.span();
    output.push_str(&format!("- {diagnostic} at {span}\n"));
}

/// Handles render.
fn render(parser: &NativeRealtimeParser, clear: bool) -> String {
    let mut output = String::new();
    if clear {
        output.push_str("\u{1b}[2J\u{1b}[H");
    }
    output.push_str("m2-syn live parser  (Ctrl-C/Ctrl-D exit, Backspace edits)\n\n");
    output.push_str("SOURCE\n");
    output.push_str(parser.source());
    if !parser.source().ends_with('\n') {
        output.push('\n');
    }

    let diagnostic_count = parser
        .cells()
        .iter()
        .map(|cell| cell.lexer_diagnostics().len())
        .sum::<usize>();
    output.push_str(&format!(
        "\nSTATE\n{:?}; {} cell(s); {} reparsed; {} lexer diagnostic(s)\n",
        parser.state(),
        parser.cells().len(),
        parser.reparsed_cells().len(),
        diagnostic_count,
    ));
    for cell in parser.cells() {
        for diagnostic in cell.lexer_diagnostics() {
            write_diagnostic(&mut output, diagnostic);
        }
    }

    output.push_str("\nTOKENS\n");
    for (index, cell) in parser.cells().iter().enumerate() {
        output.push_str(&format!(
            "cell {} · {:?} · {:?}\n",
            index + 1,
            cell.boundary(),
            cell.state()
        ));
        output.push_str(&format_pretty_cell_tokens(cell.tokens()));
    }

    output.push_str("\nSYNTAX\n");
    for (index, cell) in parser.cells().iter().enumerate() {
        output.push_str(&format!("cell {}\n", index + 1));
        if let Some(syntax) = cell.syntax() {
            output.push_str(&format_pretty_tree(syntax));
        }
        if let Some(error) = cell.parse_error() {
            output.push_str(&format!("{error}\n"));
        }
    }
    output
}

fn main() -> io::Result<()> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let _terminal = TerminalMode::raw();
    let mut bytes = Vec::new();
    let mut parser = NativeRealtimeParser::new("");
    let mut stdout = io::stdout().lock();

    if interactive {
        stdout.write_all(render(&parser, true).as_bytes())?;
        stdout.flush()?;
    }

    for byte in io::stdin().lock().bytes() {
        let byte = byte?;
        match byte {
            3 | 4 => break,
            8 | 127 => {
                if let Ok(source) = std::str::from_utf8(&bytes)
                    && let Some((index, _)) = source.char_indices().next_back()
                {
                    bytes.truncate(index);
                }
            }
            b'\r' => bytes.push(b'\n'),
            byte => bytes.push(byte),
        }
        let Ok(source) = std::str::from_utf8(&bytes) else {
            continue;
        };
        if let Some(appended) = source.strip_prefix(parser.source()) {
            parser.append(appended);
        } else if parser.source().starts_with(source) {
            parser
                .apply_edit(&TextEdit::deletion(source.len(), parser.source().len()))
                .expect("backspace always removes a valid source suffix");
        } else {
            parser = NativeRealtimeParser::new(source);
        }
        if interactive {
            stdout.write_all(render(&parser, true).as_bytes())?;
            stdout.flush()?;
        }
    }

    if !interactive {
        let source = String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        debug_assert_eq!(source, parser.source());
        stdout.write_all(render(&parser, false).as_bytes())?;
    }
    Ok(())
}
