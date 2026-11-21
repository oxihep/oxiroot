//! `name()` / `title()` and the fluent `named()` / `titled()` setters shared by
//! every named ROOT object.
//!
//! A histogram (or graph, profile, …) is just data — a binning and its contents.
//! Unlike ROOT, oxiroot never forces a name at construction and keeps no global
//! name registry, so you can build as many anonymous histograms as you like. A
//! name is only needed when you *persist* the object: it becomes the file key.
//! Set it then (or whenever) with [`named`](Hist1D::named); writing an unnamed
//! object, or two with the same name in one directory, is a loud error.

use crate::{
    Efficiency, Graph, Graph2D, MultiErrorGraph, PolyHist, Profile1D, Profile2D, Profile3D,
    SparseHist,
};

/// The four naming methods, for the body of an `impl`.
macro_rules! named_methods {
    () => {
        #[doc = "The object's name (ROOT's `fName`): the key it is written under."]
        #[must_use]
        pub fn name(&self) -> &str {
            &self.name
        }

        #[doc = "The object's title (ROOT's `fTitle`)."]
        #[must_use]
        pub fn title(&self) -> &str {
            &self.title
        }

        #[doc = "Set the object's name — the key it is written under in a ROOT"]
        #[doc = "file. Chainable: `graph.named(\"g\").titled(\"data\")`."]
        #[must_use]
        pub fn named(mut self, name: impl Into<String>) -> Self {
            self.name = name.into();
            self
        }

        #[doc = "Set the object's title (ROOT's `fTitle`)."]
        #[must_use]
        pub fn titled(mut self, title: impl Into<String>) -> Self {
            self.title = title.into();
            self
        }
    };
}

macro_rules! impl_named {
    ($($t:ty),+ $(,)?) => {$(
        impl $t {
            named_methods!();
        }
    )+};
}

impl<const D: usize> crate::HistRecord<D> {
    named_methods!();
}

impl_named!(
    Profile1D,
    Profile2D,
    Profile3D,
    Efficiency,
    SparseHist,
    PolyHist,
    Graph,
    Graph2D,
    MultiErrorGraph,
);
