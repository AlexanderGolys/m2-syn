//! Prints source, flattened tokens, and the complete typed CST for an M2 file.

use std::error::Error;
use std::fs;

use m2_syn::ParsedFile;

const SAMPLE: &str = "f = (x, y) -> if x then {y,} else {,x}\nf 1 2";

fn main() -> Result<(), Box<dyn Error>> {
    let source = match std::env::args_os().nth(1) {
        Some(path) => fs::read_to_string(path)?,
        None => SAMPLE.to_owned(),
    };
    ParsedFile::from_source(source)?.print_pretty()?;
    Ok(())
}
