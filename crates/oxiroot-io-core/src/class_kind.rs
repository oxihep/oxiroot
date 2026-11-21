//! [`ClassKind`]: what a ROOT class name says about how its objects are
//! stored, in one place.

/// What kind of object a ROOT class name names, as far as storing it goes.
///
/// Reading, merging, listing and copying all branch on the class of a key.
/// Each asking in its own way let them disagree (is a `TNtupleD` a tree? is
/// a `TH1K` a histogram?); they ask here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClassKind {
    /// A `TTree`, or a `TNtuple` / `TNtupleD`, which are trees: the key holds
    /// the tree's description, and its baskets lie elsewhere in the file.
    Tree,
    /// An `RNTuple` anchor (`ROOT::RNTuple`): its pages lie elsewhere in the
    /// file.
    Ntuple,
    /// A directory (`TDirectory`, `TDirectoryFile`).
    Directory,
    /// A histogram `TH1` / `TH2` / `TH3` of one of the six bin content types
    /// (`TH1D`, `TH2F`, `TH3I`, …), with its number of dimensions. `TH2Poly`,
    /// `TH1K` and the profiles are not: they are stored differently.
    Hist(u8),
    /// Anything else: an object held whole under its key.
    Other,
}

impl ClassKind {
    /// The kind of the class `class`.
    #[must_use]
    pub fn of(class: &str) -> Self {
        match class {
            "TTree" | "TNtuple" | "TNtupleD" => Self::Tree,
            "ROOT::RNTuple" => Self::Ntuple,
            "TDirectory" | "TDirectoryFile" => Self::Directory,
            _ => match class.as_bytes() {
                [
                    b'T',
                    b'H',
                    dim @ b'1'..=b'3',
                    b'C' | b'S' | b'I' | b'F' | b'D' | b'L',
                ] => Self::Hist(dim - b'0'),
                _ => Self::Other,
            },
        }
    }

    /// Whether an object of class `class` keeps data outside its key, so that
    /// copying the key alone would leave the data behind. This is wider than
    /// [`Tree`](Self::Tree) and [`Ntuple`](Self::Ntuple): it also covers a
    /// `TChain` and the anchor classes of older `RNTuple` versions, which
    /// this crate does not read but must not copy by their key.
    #[must_use]
    pub fn holds_data_outside_its_key(class: &str) -> bool {
        matches!(Self::of(class), Self::Tree | Self::Ntuple)
            || class == "TChain"
            || class.contains("RNTuple")
    }
}

#[cfg(test)]
mod tests {
    use super::ClassKind;

    #[test]
    fn classes_are_sorted_into_kinds() {
        for tree in ["TTree", "TNtuple", "TNtupleD"] {
            assert_eq!(ClassKind::of(tree), ClassKind::Tree);
        }
        assert_eq!(ClassKind::of("ROOT::RNTuple"), ClassKind::Ntuple);
        assert_eq!(ClassKind::of("TDirectoryFile"), ClassKind::Directory);
        assert_eq!(ClassKind::of("TH1D"), ClassKind::Hist(1));
        assert_eq!(ClassKind::of("TH2F"), ClassKind::Hist(2));
        assert_eq!(ClassKind::of("TH3L"), ClassKind::Hist(3));
        // Stored differently, whatever their names suggest.
        for other in [
            "TH2Poly",
            "TH1K",
            "TH1",
            "TH4D",
            "TProfile",
            "THnSparseT<TArrayD>",
            "",
        ] {
            assert_eq!(ClassKind::of(other), ClassKind::Other, "{other}");
        }
    }

    #[test]
    fn what_keeps_data_outside_its_key_is_wider_than_what_is_read() {
        for class in [
            "TTree",
            "TNtupleD",
            "ROOT::RNTuple",
            "TChain",
            "ROOT::Experimental::RNTuple",
        ] {
            assert!(ClassKind::holds_data_outside_its_key(class), "{class}");
        }
        assert!(!ClassKind::holds_data_outside_its_key("TH1D"));
        assert_eq!(
            ClassKind::of("ROOT::Experimental::RNTuple"),
            ClassKind::Other
        );
    }
}
