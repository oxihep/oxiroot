//! ROOT's streamer type codes (`TStreamerElement::fType`, the `EReadWrite`
//! values of `TVirtualStreamerInfo`) and the kinds of streamer element, named
//! once for the code that writes streamer info and the code that reads by it.

/// A `Char_t`.
pub const K_CHAR: i32 = 1;
/// A `Short_t`.
pub const K_SHORT: i32 = 2;
/// An `Int_t`.
pub const K_INT: i32 = 3;
/// A `Long_t`.
pub const K_LONG: i32 = 4;
/// A `Float_t`.
pub const K_FLOAT: i32 = 5;
/// An `Int_t` that counts a variable-length array member.
pub const K_COUNTER: i32 = 6;
/// A `Double_t`.
pub const K_DOUBLE: i32 = 8;
/// A `Double32_t`: a `double` in memory, by default a `float` on disk.
pub const K_DOUBLE32: i32 = 9;
/// A `UChar_t`.
pub const K_UCHAR: i32 = 11;
/// A `UShort_t`.
pub const K_USHORT: i32 = 12;
/// A `UInt_t`.
pub const K_UINT: i32 = 13;
/// A `ULong_t`.
pub const K_ULONG: i32 = 14;
/// `TObject::fBits`, a `UInt_t`.
pub const K_BITS: i32 = 15;
/// A `Long64_t`.
pub const K_LONG64: i32 = 16;
/// A `ULong64_t`.
pub const K_ULONG64: i32 = 17;
/// A `Bool_t`.
pub const K_BOOL: i32 = 18;
/// A `Float16_t`.
pub const K_FLOAT16: i32 = 19;
/// Added to a basic type's code for a fixed-length array of it (`T x[N]`).
pub const K_OFFSET_L: i32 = 20;
/// Added to a basic type's code for a counted array of it (`T* x; //[n]`).
pub const K_OFFSET_P: i32 = 40;
/// An inline object member (e.g. a `TObjArray fBranches`).
pub const K_OBJECT: i32 = 61;
/// An inline non-`TObject` member (e.g. `TIOFeatures`).
pub const K_ANY: i32 = 62;
/// A pointer to an object, written with `->` (never null).
pub const K_OBJECT_ARROW: i32 = 63;
/// A pointer to an object.
pub const K_OBJECT_PTR: i32 = 64;
/// A `TString` member.
pub const K_TSTRING: i32 = 65;
/// The `TObject` base.
pub const K_TOBJECT: i32 = 66;
/// The `TNamed` base.
pub const K_TNAMED: i32 = 67;
/// A pointer to a non-`TObject` class, written with `->`.
pub const K_ANY_ARROW: i32 = 68;
/// A pointer to a non-`TObject` class (`TStreamerObjectAnyPointer`).
pub const K_ANY_PTR: i32 = 69;
/// The first of the STL collection codes (`kSTL` 300 plus the collection kind).
pub const K_STL_FIRST: i32 = 300;
/// A `std::string` (`kSTLstring`), the last of the STL codes.
pub const K_STL_STRING: i32 = 365;
/// An STL container member (`TStreamerSTL`, e.g. `vector<double>`).
pub const K_STL: i32 = 500;
/// A counted array of objects (`TStreamerLoop`).
pub const K_STREAM_LOOP: i32 = 501;

/// What kind of member a streamer element describes: the `TStreamerElement`
/// subclass ROOT wrote it as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ElementKind {
    /// `TStreamerBase`: a base class.
    Base,
    /// `TStreamerBasicType`: a number, or a fixed array of them.
    BasicType,
    /// `TStreamerBasicPointer`: a counted array of numbers.
    BasicPointer,
    /// `TStreamerLoop`: a counted array of objects.
    Loop,
    /// `TStreamerObject`: an inline `TObject`.
    Object,
    /// `TStreamerObjectAny`: an inline object of another class.
    ObjectAny,
    /// `TStreamerObjectPointer`: a pointer to a `TObject`.
    ObjectPointer,
    /// `TStreamerObjectAnyPointer`: a pointer to an object of another class.
    ObjectAnyPointer,
    /// `TStreamerString`: a `TString`.
    String,
    /// `TStreamerSTL`: an STL container.
    Stl,
    /// `TStreamerSTLstring`: a `std::string`.
    StlString,
    /// Any other subclass.
    Other,
}

impl ElementKind {
    /// The kind of the `TStreamerElement` subclass `class`.
    #[must_use]
    pub fn of(class: &str) -> Self {
        match class {
            "TStreamerBase" => Self::Base,
            "TStreamerBasicType" => Self::BasicType,
            "TStreamerBasicPointer" => Self::BasicPointer,
            "TStreamerLoop" => Self::Loop,
            "TStreamerObject" => Self::Object,
            "TStreamerObjectAny" => Self::ObjectAny,
            "TStreamerObjectPointer" => Self::ObjectPointer,
            "TStreamerObjectAnyPointer" => Self::ObjectAnyPointer,
            "TStreamerString" => Self::String,
            "TStreamerSTL" => Self::Stl,
            "TStreamerSTLstring" => Self::StlString,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ElementKind;

    #[test]
    fn element_classes_have_kinds() {
        assert_eq!(ElementKind::of("TStreamerBase"), ElementKind::Base);
        assert_eq!(
            ElementKind::of("TStreamerSTLstring"),
            ElementKind::StlString
        );
        assert_eq!(ElementKind::of("TStreamerLoop"), ElementKind::Loop);
        assert_eq!(ElementKind::of("TStreamerArtificial"), ElementKind::Other);
    }
}
