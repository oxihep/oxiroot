//! Collection objects holding other objects: [`HistStack`] (a stack of
//! histograms) and [`GraphStack`] (several graphs drawn together). Both store
//! their members in a `TList` and serialize byte-for-byte as ROOT does, so ROOT
//! and uproot read what oxiroot writes and vice versa.
//!
//! The members are written through the generic object protocol — each one with
//! a fresh class tag (`kNewClassTag`), never a back-reference — so writing is
//! position-independent. Reading uses [`TagReader`], which resolves both the
//! class tags oxiroot writes and the back-references ROOT writes.

use oxiroot_io_core::cast::SaturatingFrom;
use oxiroot_io_core::{Object, ReadRoot};
use std::borrow::Cow;
use std::ops::Range;

use oxiroot_io_core::raw::{
    TagReader, read_object_base, write_named, write_object_any, write_object_base,
};
use oxiroot_io_core::streamer_gen::{ClassDef, base, basic, object_ptr};
use oxiroot_io_core::{Error, RBuffer, Result, StreamerSet, WBuffer};

use crate::base::bin_content_type_of;
use crate::graph::{Graph, read_tgraph};
use crate::record::Hist1D;
use crate::threaded::Mergeable;
use crate::write::{WriteRoot, hist_streamer_classes};

/// HistStack/GraphStack leave `fMaximum`/`fMinimum` at this sentinel until drawn.
const UNSET_LIMIT: f64 = -1111.0;

// --- streamer info -----------------------------------------------------------
//
// uproot models a HistStack or GraphStack only from its streamer, so files that
// store one embed these entries (versions and checksums as ROOT writes them).

/// The `TStreamerInfo` of `HistStack`.
fn thstack_class() -> ClassDef<'static> {
    ClassDef {
        name: "THStack".into(),
        version: 2,
        checksum: 1_918_797_077,
        elements: vec![
            base("TNamed", 1),
            object_ptr("fHists", "TList*"),
            object_ptr("fHistogram", "TH1*"),
            basic("fMaximum", 8, 8, "double"),
            basic("fMinimum", 8, 8, "double"),
        ],
    }
}

/// The `TStreamerInfo` of `GraphStack`.
fn tmultigraph_class() -> ClassDef<'static> {
    ClassDef {
        name: "TMultiGraph".into(),
        version: 2,
        checksum: 3_767_090_389,
        elements: vec![
            base("TNamed", 1),
            object_ptr("fGraphs", "TList*"),
            object_ptr("fFunctions", "TList*"),
            object_ptr("fHistogram", "TH1F*"),
            basic("fMaximum", 8, 8, "double"),
            basic("fMinimum", 8, 8, "double"),
        ],
    }
}

// --- shared object-protocol helpers -----------------------------------------

/// Write a `TList*` member named `list_name` holding `members` (each a
/// `(class, body)` pair), wrapped as a `TList` object via [`write_object_any`].
fn write_object_list(w: &mut WBuffer, list_name: &str, members: &[(String, Vec<u8>)]) {
    let mut body = WBuffer::new();
    let list = body.begin_object(5); // TList version 5
    write_object_base(&mut body, 0);
    body.string(list_name); // fName
    body.be_i32(i32::saturating_from(members.len())); // nobjects
    for (class, member) in members {
        write_object_any(&mut body, class, member);
        body.string(""); // the per-object option string
    }
    body.end_object(list);
    write_object_any(w, "TList", &body.into_vec());
}

/// Read a `Named` base (version header, `TObject`, `fName`, `fTitle`).
fn read_named(r: &mut RBuffer<'_>) -> Result<(String, String)> {
    r.read_version()?;
    read_object_base(r)?;
    let name = r.string()?;
    let title = r.string()?;
    Ok((name, title))
}

/// Open a `TList*` member and return, for each entry, its class name and the
/// byte range of its body (the bytes one can hand to a per-class decoder). A
/// null member pointer (an absent list) yields an empty vector. Reuses
/// [`TagReader`] so ROOT's class back-references resolve.
fn list_member_ranges(
    r: &mut RBuffer<'_>,
    tags: &mut TagReader,
) -> Result<Vec<(String, Range<usize>)>> {
    let (list_end, n) = tags.open_list(r)?;
    // `n` comes from the file; every entry takes at least a 4-byte header.
    let mut out = Vec::with_capacity(r.bounded_capacity(n, 4));
    for _ in 0..n {
        let header = tags.read_header(r)?;
        match (header.class_name, header.end) {
            (Some(class), Some(end)) => {
                out.push((class.to_string(), r.pos()..end));
                r.seek(end)?;
            }
            (_, Some(end)) => r.seek(end)?, // a null/parent slot
            _ => {}
        }
        r.skip_string()?; // the per-object option string
    }
    if let Some(end) = list_end {
        r.seek(end)?;
    }
    Ok(out)
}

// --- Stack ------------------------------------------------------------------

/// A named group of histograms or graphs, drawn in one frame: ROOT's
/// `THStack` for histograms, `TMultiGraph` for graphs. Use it through
/// [`HistStack`] and [`GraphStack`]. Build one with [`Stack::new`], name it
/// with [`named`](Stack::named), and [`add`](Stack::add) the members; write
/// it through [`FileWriter`](oxiroot_io_core::FileWriter).
#[derive(Debug, Clone, PartialEq)]
pub struct Stack<T: StackMember> {
    name: String,
    title: String,
    members: Vec<T>,
}

/// A stack of histograms, ROOT's `THStack`.
#[doc(alias = "THStack")]
pub type HistStack = Stack<Hist1D>;
/// Several graphs in one frame, ROOT's `TMultiGraph`.
#[doc(alias = "TMultiGraph")]
pub type GraphStack = Stack<Graph>;

mod sealed {
    pub trait Sealed {}
    impl Sealed for crate::Hist1D {}
    impl Sealed for crate::Graph {}
}

/// What a [`Stack`] can hold: [`Hist1D`] and [`Graph`]. It says which ROOT
/// class stacks the type and how one member is read; it cannot be
/// implemented outside this crate.
pub trait StackMember: WriteRoot + Sized + sealed::Sealed {
    /// The ROOT class of a stack of these.
    const STACK_CLASS: &'static str;

    /// The streamer info of that class.
    fn stack_class() -> ClassDef<'static>;

    /// Write the stack's members that follow its list: the null pointers,
    /// then the unset `fMaximum` and `fMinimum`.
    fn write_stack_tail(w: &mut WBuffer);

    /// Read the member of class `class` at the cursor, which shares the
    /// stack's buffer and so its class tags. `None` if the stack does not
    /// hold that class as a member.
    fn read_member(
        r: &mut RBuffer<'_>,
        stack: &str,
        class: &str,
        tags: &mut TagReader,
    ) -> Option<Result<Self>>;
}

impl StackMember for Hist1D {
    const STACK_CLASS: &'static str = "THStack";

    fn stack_class() -> ClassDef<'static> {
        thstack_class()
    }

    fn write_stack_tail(w: &mut WBuffer) {
        w.be_u32(0); // fHistogram (null TH1*)
        w.be_f64(UNSET_LIMIT); // fMaximum
        w.be_f64(UNSET_LIMIT); // fMinimum
    }

    fn read_member(
        r: &mut RBuffer<'_>,
        _stack: &str,
        class: &str,
        tags: &mut TagReader,
    ) -> Option<Result<Self>> {
        class
            .starts_with("TH1")
            .then(|| Self::read(r, bin_content_type_of(class)?, Some(tags)))
    }
}

impl StackMember for Graph {
    const STACK_CLASS: &'static str = "TMultiGraph";

    fn stack_class() -> ClassDef<'static> {
        tmultigraph_class()
    }

    fn write_stack_tail(w: &mut WBuffer) {
        w.be_u32(0); // fFunctions (null TList*)
        w.be_u32(0); // fHistogram (null TH1F*)
        w.be_f64(UNSET_LIMIT); // fMaximum
        w.be_f64(UNSET_LIMIT); // fMinimum
    }

    fn read_member(
        r: &mut RBuffer<'_>,
        stack: &str,
        class: &str,
        tags: &mut TagReader,
    ) -> Option<Result<Self>> {
        class
            .starts_with("TGraph")
            .then(|| read_tgraph(r, stack, class, Some(tags)))
    }
}

impl<T: StackMember> Default for Stack<T> {
    fn default() -> Self {
        Self {
            name: String::new(),
            title: String::new(),
            members: Vec::new(),
        }
    }
}

impl<T: StackMember> Stack<T> {
    /// An empty stack (give it a key name with [`named`](Self::named)).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the key name this stack is stored under.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the stack's title.
    #[must_use]
    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Add a member to the stack.
    // `add` is the natural builder verb here; it is not the arithmetic `Add::add`.
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn add(mut self, member: T) -> Self {
        self.members.push(member);
        self
    }

    /// The key name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The members, in the order they were added.
    #[must_use]
    pub fn members(&self) -> &[T] {
        &self.members
    }

    fn decode(object: &[u8], key_len: usize) -> Result<Self> {
        let mut r = RBuffer::new(object);
        r.read_version()?;
        let (name, title) = read_named(&mut r)?;
        let mut tags = TagReader::new(key_len);
        let ranges = list_member_ranges(&mut r, &mut tags)?;
        let mut members = Vec::with_capacity(ranges.len());
        for (class, range) in ranges {
            // Read in place, with the map the list was walked with: the
            // members share one buffer, and so one set of class tags.
            let mut member = RBuffer::new(object);
            member.seek(range.start)?;
            if let Some(read) = T::read_member(&mut member, &name, &class, &mut tags) {
                members.push(read?);
            }
        }
        Ok(Self {
            name,
            title,
            members,
        })
    }
}

impl Mergeable for Stack<Hist1D> {
    type Error = oxiroot_io_core::Error;

    /// Merge the stacks' histograms by name, as ROOT's `THStack::Merge` and
    /// `hadd` do: a histogram both stacks hold is summed, one only `other`
    /// holds is appended.
    ///
    /// Returns [`oxiroot_io_core::Error::BinningMismatch`] if two histograms of
    /// the same name have different binnings; the histograms merged before it
    /// keep their sums.
    fn merge(&mut self, other: &Self) -> Result<()> {
        for from in &other.members {
            match self.members.iter_mut().find(|h| h.name == from.name) {
                Some(h) => h.add(from, 1.0)?,
                None => self.members.push(from.clone()),
            }
        }
        Ok(())
    }
}

impl<T: StackMember> WriteRoot for Stack<T> {
    fn root_class(&self) -> Cow<'static, str> {
        Cow::Borrowed(T::STACK_CLASS)
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        &self.title
    }
    fn write_body(&self, w: &mut WBuffer) {
        let obj = w.begin_object(2); // THStack and TMultiGraph are both version 2
        write_named(w, 0, &self.name, &self.title);
        let members: Vec<(String, Vec<u8>)> = (self.members.iter())
            .map(|m| (m.root_class().into_owned(), m.to_root_bytes()))
            .collect();
        write_object_list(w, "", &members);
        T::write_stack_tail(w);
        w.end_object(obj);
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        // Its bases and list, the members' classes, then its own.
        let mut set = StreamerSet::default();
        set.add_classes(hist_streamer_classes(&["TNamed", "TList"]));
        for member in &self.members {
            set.add(member);
        }
        set.add_classes([T::stack_class()]);
        set.into_classes()
    }
}

impl<T: StackMember> ReadRoot for Stack<T> {
    const CLASS: &'static str = T::STACK_CLASS;

    fn from_object(object: &Object<'_>) -> Option<Result<Self>> {
        (object.class == Self::CLASS).then(|| {
            let key_len = object.key_len.ok_or_else(|| {
                Error::Unsupported(format!(
                    "a {} inside a collection cannot be read",
                    T::STACK_CLASS
                ))
            })?;
            Self::decode(object.bytes, key_len)
        })
    }
}
