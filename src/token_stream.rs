use std::fmt::{Display, Formatter};

use crate::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delimiter {
    Parenthesis,
    Bracket,
    Brace,
    AngleBar,
}

impl Delimiter {
    fn open(self) -> &'static str {
        match self {
            Self::Parenthesis => "(",
            Self::Bracket => "[",
            Self::Brace => "{",
            Self::AngleBar => "<|",
        }
    }

    fn close(self) -> &'static str {
        match self {
            Self::Parenthesis => ")",
            Self::Bracket => "]",
            Self::Brace => "}",
            Self::AngleBar => "|>",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenTree {
    Text {
        text: String,
        span: Span,
    },
    Group {
        delimiter: Delimiter,
        stream: TokenStream,
        span: Span,
    },
    EndOfCell(Span),
    EndOfFile(Span),
}

impl TokenTree {
    fn render(&self, output: &mut String) {
        match self {
            Self::Text { text, .. } => output.push_str(text),
            Self::Group {
                delimiter, stream, ..
            } => {
                output.push_str(delimiter.open());
                stream.render(output);
                output.push_str(delimiter.close());
            }
            Self::EndOfCell(_) => output.push('\n'),
            Self::EndOfFile(_) => {}
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenStream(Vec<TokenTree>);

impl TokenStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_text(&mut self, text: impl Into<String>, span: Span) {
        self.0.push(TokenTree::Text {
            text: text.into(),
            span,
        });
    }

    pub fn push_synthetic(&mut self, text: impl Into<String>) {
        self.push_text(text, Span::detached());
    }

    pub fn push_space(&mut self) {
        self.push_synthetic(" ");
    }

    pub fn push_group(&mut self, delimiter: Delimiter, stream: Self, span: Span) {
        self.0.push(TokenTree::Group {
            delimiter,
            stream,
            span,
        });
    }

    pub fn push_end_of_cell(&mut self, span: Span) {
        self.0.push(TokenTree::EndOfCell(span));
    }

    pub fn push_end_of_file(&mut self, span: Span) {
        self.0.push(TokenTree::EndOfFile(span));
    }

    pub fn iter(&self) -> impl Iterator<Item = &TokenTree> {
        self.0.iter()
    }

    pub fn render(&self, output: &mut String) {
        for token in &self.0 {
            token.render(output);
        }
    }
}

impl Display for TokenStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let mut source = String::new();
        self.render(&mut source);
        formatter.write_str(&source)
    }
}

pub trait ToTokens {
    fn to_tokens(&self, output: &mut TokenStream);

    fn tokens(&self) -> TokenStream {
        let mut output = TokenStream::new();
        self.to_tokens(&mut output);
        output
    }

    fn to_m2(&self) -> String {
        self.tokens().to_string()
    }
}

impl<T> ToTokens for &T
where
    T: ToTokens + ?Sized,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        (*self).to_tokens(output);
    }
}

impl<T> ToTokens for Box<T>
where
    T: ToTokens + ?Sized,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        self.as_ref().to_tokens(output);
    }
}

impl<T> ToTokens for Option<T>
where
    T: ToTokens,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        if let Some(value) = self {
            value.to_tokens(output);
        }
    }
}

impl ToTokens for TokenStream {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.extend(self.0.iter().cloned());
    }
}
