//! Values produced from incomplete source together with recoverable diagnostics.

/// A successfully produced value accompanied by non-fatal diagnostics.
///
/// Recovery is explicit rather than hidden in ordinary `Result` values: strict
/// entry points may reject the same source, while real-time callers can retain
/// the usable syntax in [`Self::value`] and present [`Self::diagnostics`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered<T, D> {
    /// The value retained from the incomplete input.
    pub value: T,
    /// Non-fatal problems encountered while producing the value.
    pub diagnostics: Vec<D>,
}

impl<T, D> Recovered<T, D> {
    /// Creates a recovered value and its diagnostics.
    pub fn new(value: T, diagnostics: Vec<D>) -> Self {
        Self { value, diagnostics }
    }

    /// Separates the recovered value from its diagnostics.
    pub fn into_parts(self) -> (T, Vec<D>) {
        (self.value, self.diagnostics)
    }

    /// Transforms the recovered value without changing its diagnostics.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Recovered<U, D> {
        Recovered::new(map(self.value), self.diagnostics)
    }
}
