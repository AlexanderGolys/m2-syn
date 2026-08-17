extern crate self as m2_syn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span(u8, u8);

impl Span {
    pub fn detached() -> Self {
        Self::default()
    }

    pub fn join(self, other: Self) -> Self {
        Self(self.0.min(other.0), self.1.max(other.1))
    }

    pub fn join_all(spans: impl IntoIterator<Item = Self>) -> Self {
        spans.into_iter().reduce(Self::join).unwrap_or_default()
    }
}

pub trait Spanned {
    fn span(&self) -> Span;
}

impl Spanned for Span {
    fn span(&self) -> Span {
        *self
    }
}

struct NotSpanned;

#[derive(m2_syn_macros::Spanned)]
enum SpannedVariants {
    Named { marker: NotSpanned, span: Span },
    Tuple(Span, Span),
    Unit,
}

#[test]
fn enum_variants_use_the_same_member_span_rules_as_structs() {
    let named_span = Span(2, 4);
    let named = SpannedVariants::Named {
        marker: NotSpanned,
        span: named_span,
    };
    let tuple = SpannedVariants::Tuple(Span(3, 5), Span(8, 13));

    assert_eq!(named.span(), named_span);
    assert_eq!(tuple.span(), Span(3, 13));
    assert_eq!(SpannedVariants::Unit.span(), Span::detached());
}
