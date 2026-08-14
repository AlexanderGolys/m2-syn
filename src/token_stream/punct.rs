use std::fmt::{Display, Formatter, Write};

#[allow(unused_macros)]
macro_rules! format_punct_char {
    [.]   => ("Dot");
    [,]   => ("Cma");
    [;]   => ("Scl");
    [:]   => ("Col");
    [#]   => ("Hsh");
    [@]   => ("Att");
    [%]   => ("Mod");
    [^]   => ("Crt");
    [&]   => ("Amp");
    [*]   => ("Mul");
    [+]   => ("Add");
    [-]   => ("Sub");
    [=]   => ("Eql");
    [<]   => ("Lst");
    [>]   => ("Gst");
    [!]   => ("Bng");
    [?]   => ("Qsm");
    [~]   => ("Tld");
    [|]   => ("Pip");
    [/]   => ("Slh");
    [_]   => ("Ubs");
    ["\\"]  => ("Bsl");
    ["·"]   => ("Cdt");
    ["⊠"] => ("Box");
    ["⧢"] => ("Sfp");
    ["("]   => ("Lpr");
    [")"]   => ("Rpr");
    ["["]   => ("Lbr");
    ["]"]   => ("Rbr");
    ["{"]   => ("Lbc");
    ["]"]   => ("Rbc");
    []   => ("Adj");
    [" "]   => ("Adj");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PunctChar {
    Dot, // '.'
    Cma, // ','
    Scl, // ';'
    Col, // ':'
    Hsh, // '#'
    Att, // '@'
    Mod, // '%'
    Crt, // '^'
    Amp, // '&'
    Mul, // '*'
    Add, // '+'
    Sub, // '-'
    Eql, // '='
    Lst, // '<'
    Gst, // '>'
    Bng, // '!'
    Qsm, // '?'
    Tld, // '~'
    Pip, // '|'
    Slh, // '/'
    Ubs, // '_'
    Bsl, // '\\'
    Cdt, // '·'
    Box, // '⊠'
    Sfp, // '⧢'

    Lpr, // '('
    Rpr, // ')'
    Lbr, // '['
    Rbr, // ']'
    Lbc, // '{'
    Rbc, // '}'

    Adj, // ' '
}

impl PunctChar {
    fn as_char(&self) -> char {
        match self {
            Self::Dot => '.',
            Self::Cma => ',',
            Self::Scl => ';',
            Self::Col => ':',
            Self::Hsh => '#',
            Self::Att => '@',
            Self::Mod => '%',
            Self::Crt => '^',
            Self::Amp => '&',
            Self::Mul => '*',
            Self::Add => '+',
            Self::Sub => '-',
            Self::Eql => '=',
            Self::Lst => '<',
            Self::Gst => '>',
            Self::Bng => '!',
            Self::Qsm => '?',
            Self::Tld => '~',
            Self::Pip => '|',
            Self::Slh => '/',
            Self::Ubs => '_',
            Self::Bsl => '\\',
            Self::Cdt => '·',
            Self::Box => '⊠',
            Self::Sfp => '⧢',
            Self::Lpr => '(',
            Self::Rpr => ')',
            Self::Lbr => '[',
            Self::Rbr => ']',
            Self::Lbc => '{',
            Self::Rbc => '}',
            Self::Adj => ' ',
        }
    }
}

impl TryFrom<char> for PunctChar {
    type Error = ();

    fn try_from(value: char) -> Result<Self, Self::Error> {
        match value {
            '.' => Ok(Self::Dot),
            ',' => Ok(Self::Cma),
            ';' => Ok(Self::Scl),
            ':' => Ok(Self::Col),
            '#' => Ok(Self::Hsh),
            '@' => Ok(Self::Att),
            '%' => Ok(Self::Mod),
            '^' => Ok(Self::Crt),
            '&' => Ok(Self::Amp),
            '*' => Ok(Self::Mul),
            '+' => Ok(Self::Add),
            '-' => Ok(Self::Sub),
            '=' => Ok(Self::Eql),
            '<' => Ok(Self::Lst),
            '>' => Ok(Self::Gst),
            '!' => Ok(Self::Bng),
            '?' => Ok(Self::Qsm),
            '~' => Ok(Self::Tld),
            '|' => Ok(Self::Pip),
            '/' => Ok(Self::Slh),
            '_' => Ok(Self::Ubs),
            '\\' => Ok(Self::Bsl),
            '·' => Ok(Self::Cdt),
            '⊠' => Ok(Self::Box),
            '⧢' => Ok(Self::Sfp),
            '(' => Ok(Self::Lpr),
            ')' => Ok(Self::Rpr),
            '[' => Ok(Self::Lbr),
            ']' => Ok(Self::Rbr),
            '{' => Ok(Self::Lbc),
            '}' => Ok(Self::Rbc),
            ' ' => Ok(Self::Adj),
            _ => Err(()),
        }
    }
}

/// Spacing between punctuation atoms, unlike in Rust, may differ between
/// tokens split between separate lines and inline.
/// # Example
/// ```macaulay2
/// -- Joint 2 postfixes, same effect to inline separated
/// 1!! == 1! !
///
/// -- Joint postfix + adjacency vs infix + prefix when separated
/// 1_~ 1
/// 1 _ ~1
///
/// -- Inline separated postfixes on global scope no longer work when  separated
/// -- by a line break.
/// 1! !    -- OK
/// (1! !)  -- OK
///
/// 1!
/// !       -- Error
///
///(1!
/// !)      -- OK
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Spacing {
    LineBreak,
    #[default]
    Whitespace,
    Joint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Punct {
    pub ch: PunctChar,
    spacing: Spacing,
}

impl Punct {
    pub fn new(ch: PunctChar, spacing: Spacing) -> Self {
        Self { ch, spacing }
    }

    pub fn try_from_char(ch: char, spacing: Spacing) -> Option<Self> {
        let Ok(c) = PunctChar::try_from(ch) else {
            return None;
        };
        Some(Self::new(c, spacing))
    }

    pub fn as_char(&self) -> char {
        self.ch.as_char()
    }

    pub fn spacing(&self) -> Spacing {
        self.spacing
    }
}

impl Display for Punct {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        formatter.write_char(self.ch.as_char())
    }
}
