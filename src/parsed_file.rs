//! Convenient ownership and projection of one parsed Macaulay2 source file.
//!
//! [`ParsedFile`] is the high-level entry point for applications that do not
//! need to assemble the lexer and parser pipeline themselves. It owns the
//! generated typed [`SourceFile`] and exposes fresh projections of that tree as
//! tokens, cells, normalized source, or indented debug output. The typed CST is
//! the sole mutable representation, so changing it cannot leave a cached token
//! stream out of sync.

use std::fmt::{Display, Formatter};
#[cfg(feature = "pretty")]
use std::io;

#[cfg(feature = "pretty")]
use crate::pretty::{
    PrettyOptions, PrettyReport, PrettyTree, format_tokens, format_tree, print_report,
};
use crate::{
    CellStream, Lex, NativeParseError, SourceFile, Span, Spanned, TokenStream, parse_native,
};
#[cfg(feature = "tree-sitter")]
use crate::{ParseError, parse_file};

/// An owned typed CST together with the original input.
///
/// With the `tree-sitter` feature, `from_source` selects that adapter;
/// [`ParsedFile::from_source_native`] always selects the direct precedence
/// parser. After parsing, callers may mutate [`Self::cst_mut`] or use
/// [`Self::edit`], then project the resulting tree through
/// [`Self::token_stream`] or [`Self::to_source`].
///
/// The original string is retained as an immutable snapshot for diagnostics.
/// Cell and source-file comment ownership is retained by the typed graph;
/// source-backed layout is recovered from spans during emission.
#[derive(Debug)]
pub struct ParsedFile {
    /// The original source.
    original_source: String,
    /// The CST.
    cst: SourceFile,
}

impl ParsedFile {
    /// Parses source with the Tree-sitter adapter.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when Tree-sitter rejects the source or typed
    /// migration fails.
    #[cfg(feature = "tree-sitter")]
    pub fn from_source(source: impl Into<String>) -> Result<Self, ParseError> {
        let original_source = source.into();
        let cst = parse_file(&original_source)?;
        Ok(Self {
            original_source,
            cst,
        })
    }

    /// Parses source with the native precedence parser.
    ///
    /// # Errors
    ///
    /// Returns [`NativeParseError`] when lexing or native parsing fails.
    pub fn from_source_native(source: impl Into<String>) -> Result<Self, NativeParseError> {
        let original_source = source.into();
        let cst = parse_native(&original_source)?;
        Ok(Self {
            original_source,
            cst,
        })
    }

    /// Returns the input snapshot exactly as supplied to the parser.
    pub fn original_source(&self) -> &str {
        &self.original_source
    }

    /// Borrows the generated typed CST root.
    pub fn cst(&self) -> &SourceFile {
        &self.cst
    }

    /// Mutably borrows the generated typed CST root.
    pub fn cst_mut(&mut self) -> &mut SourceFile {
        &mut self.cst
    }

    /// Applies an edit and returns the file for further method chaining.
    pub fn edit(mut self, edit: impl FnOnce(&mut SourceFile)) -> Self {
        edit(&mut self.cst);
        self
    }

    /// Consumes the facade and returns its generated typed CST.
    pub fn into_cst(self) -> SourceFile {
        self.cst
    }

    /// Flattens the current CST into one token stream.
    pub fn token_stream(&self) -> TokenStream {
        self.cst.to_token_stream()
    }

    /// Projects the current CST as independently parseable cells.
    ///
    /// This is the normal macro-facing representation after tokenisation. Use
    /// [`Self::token_stream`] when a function-like transformation intentionally
    /// needs the less structured lexical representation.
    pub fn cell_stream(&self) -> CellStream {
        CellStream::from_source_file(&self.cst)
    }

    /// Emits the current CST as normalized Macaulay2 source.
    pub fn to_source(&self) -> String {
        self.cell_stream().to_string()
    }

    /// Formats the current raw token representation as a recursively flattened
    /// table with explicit group boundaries.
    #[cfg(feature = "pretty")]
    pub fn pretty_tokens(&self) -> String {
        format_tokens(&self.token_stream(), PrettyOptions::PLAIN)
    }

    /// Formats the complete generated typed CST as a Unicode tree.
    #[cfg(feature = "pretty")]
    pub fn pretty_cst(&self) -> String {
        format_tree(&self.cst.pretty_tree(), PrettyOptions::PLAIN)
    }

    /// Builds a combined source, flat-token, and typed-CST inspection report.
    #[cfg(feature = "pretty")]
    pub fn pretty_report(&self) -> PrettyReport<'_> {
        PrettyReport::new(
            &self.original_source,
            self.token_stream(),
            self.cst.pretty_tree(),
        )
    }

    /// Prints the combined report to standard output, enabling ANSI styling
    /// only when standard output is an interactive terminal.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when standard output cannot be written.
    #[cfg(feature = "pretty")]
    pub fn print_pretty(&self) -> io::Result<()> {
        print_report(self.pretty_report())
    }
}

impl AsRef<SourceFile> for ParsedFile {
    fn as_ref(&self) -> &SourceFile {
        self.cst()
    }
}

impl AsMut<SourceFile> for ParsedFile {
    fn as_mut(&mut self) -> &mut SourceFile {
        self.cst_mut()
    }
}

impl Display for ParsedFile {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.cell_stream().fmt(formatter)
    }
}

impl Spanned for ParsedFile {
    fn span(&self) -> Span {
        self.cst.span()
    }
}

impl Lex for ParsedFile {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.cst.to_tokens(output);
    }
}
