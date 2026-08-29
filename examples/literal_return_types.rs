//! Demonstrates literal return types.

use std::env;
use std::error::Error;
use std::fs;
use std::io::{Error as IoError, ErrorKind};

use m2_syn::{CellStream, LiteralReturnTypes, ProcMacro, lex_str};

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os().nth(1).ok_or_else(|| {
        IoError::new(
            ErrorKind::InvalidInput,
            "usage: cargo run --example literal_return_types -- <source.m2>",
        )
    })?;
    let source = fs::read_to_string(path)?;
    let input = CellStream::new(lex_str(&source)?)?;
    let output = LiteralReturnTypes.expand(input, &())?;
    print!("{output}");
    Ok(())
}
