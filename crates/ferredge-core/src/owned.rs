extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};

/// Converts a borrowing model type into its fully owned counterpart.
pub trait IntoOwned {
    /// Fully owned counterpart of this type.
    type Owned: 'static;
    /// Materializes an owned value for async or persistence boundaries.
    fn into_owned(self) -> Self::Owned;
}

impl IntoOwned for Cow<'_, str> {
    type Owned = Cow<'static, str>;
    fn into_owned(self) -> Cow<'static, str> {
        Cow::Owned(String::from(self))
    }
}

impl IntoOwned for Cow<'_, [u8]> {
    type Owned = Cow<'static, [u8]>;
    fn into_owned(self) -> Cow<'static, [u8]> {
        Cow::Owned(Vec::from(self))
    }
}

impl<T: IntoOwned> IntoOwned for Option<T> {
    type Owned = Option<T::Owned>;
    fn into_owned(self) -> Self::Owned {
        self.map(IntoOwned::into_owned)
    }
}

impl<T: IntoOwned> IntoOwned for Vec<T> {
    type Owned = Vec<T::Owned>;
    fn into_owned(self) -> Self::Owned {
        self.into_iter().map(IntoOwned::into_owned).collect()
    }
}

impl<A: IntoOwned, B: IntoOwned> IntoOwned for (A, B) {
    type Owned = (A::Owned, B::Owned);
    fn into_owned(self) -> Self::Owned {
        (IntoOwned::into_owned(self.0), IntoOwned::into_owned(self.1))
    }
}
