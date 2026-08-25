use std::io::{self, IsTerminal, Read, Write};
use std::process::Command;

use m2_syn::{
    LexError, Spanned, TokenTree, format_pretty_tokens, format_pretty_tree, lex_str_recovering,
    parse_native_recovering,
};

struct TerminalMode(Option<String>);

impl TerminalMode {
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

fn write_diagnostic(output: &mut String, diagnostic: &LexError) {
    let span = diagnostic.span();
    output.push_str(&format!("- {diagnostic} at {:?}\n", span.bounds()));
}

fn render(source: &str, clear: bool) -> String {
    let mut output = String::new();
    if clear {
        output.push_str("\u{1b}[2J\u{1b}[H");
    }
    output.push_str("m2-syn live parser  (Ctrl-C/Ctrl-D exit, Backspace edits)\n\n");
    output.push_str("SOURCE\n");
    output.push_str(source);
    if !source.ends_with('\n') {
        output.push('\n');
    }

    let recovered_tokens = match lex_str_recovering(source) {
        Ok(tokens) => tokens,
        Err(error) => {
            output.push_str(&format!("\nSTATE\nlexical error: {error}\n"));
            return output;
        }
    };
    let cell_count = recovered_tokens
        .value
        .iter()
        .filter(|token| matches!(token, TokenTree::Eoc(_)))
        .count();
    output.push_str(&format!(
        "\nSTATE\n{} lexer cell(s); {} recoverable diagnostic(s)\n",
        cell_count,
        recovered_tokens.diagnostics.len()
    ));
    for diagnostic in &recovered_tokens.diagnostics {
        write_diagnostic(&mut output, diagnostic);
    }

    output.push_str("\nTOKENS\n");
    output.push_str(&format_pretty_tokens(&recovered_tokens.value));

    output.push_str("\nSYNTAX\n");
    match parse_native_recovering(source) {
        Ok(parsed) => {
            if parsed.diagnostics.is_empty() {
                output.push_str("complete\n");
            } else {
                output.push_str("incomplete but structurally recovered\n");
            }
            output.push_str(&format_pretty_tree(&parsed.value));
        }
        Err(error) => {
            output.push_str(&format!("expected parser state: {error}\n"));
        }
    }
    output
}

fn main() -> io::Result<()> {
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let _terminal = TerminalMode::raw();
    let mut bytes = Vec::new();
    let mut stdout = io::stdout().lock();

    if interactive {
        stdout.write_all(render("", true).as_bytes())?;
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
        if interactive {
            stdout.write_all(render(source, true).as_bytes())?;
            stdout.flush()?;
        }
    }

    if !interactive {
        let source = String::from_utf8(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        stdout.write_all(render(&source, false).as_bytes())?;
    }
    Ok(())
}
