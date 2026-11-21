//! A collection read from a file keeps the streamer info that file stores for
//! its members — and for the classes they depend on — so that, written again,
//! it still describes members of classes this crate knows nothing about.

use oxiroot_io_core::raw::{StreamerInfo, write_named};
use oxiroot_io_core::streamer_gen::{ClassDef, any, base, basic};
use oxiroot_io_core::{
    Compression, FileReader, FileWriter, ObjList, ObjMap, ObjString, WBuffer, WriteRoot,
};
use std::borrow::Cow;

/// An object of a class only this test knows: `MyEvent` (version 3), a `Named`
/// with an `int` and a `MyHit` member. Its streamer info comes with it.
struct MyEvent {
    name: String,
}

fn my_hit() -> ClassDef<'static> {
    ClassDef {
        name: "MyHit".into(),
        version: 1,
        checksum: 0x1111,
        elements: vec![basic("fE", 8, 8, "double")],
    }
}

fn my_event() -> ClassDef<'static> {
    ClassDef {
        name: "MyEvent".into(),
        version: 3,
        checksum: 0x3333,
        elements: vec![
            base("TNamed", 1),
            basic("fRun", 3, 4, "int"),
            any("fHit", 8, "MyHit"),
        ],
    }
}

impl WriteRoot for MyEvent {
    fn root_class(&self) -> Cow<'static, str> {
        Cow::Borrowed("MyEvent")
    }
    fn root_name(&self) -> &str {
        &self.name
    }
    fn root_title(&self) -> &str {
        ""
    }
    fn write_body(&self, w: &mut WBuffer) {
        let event = w.begin_object(3);
        write_named(w, 0, &self.name, "");
        w.be_i32(7); // fRun
        let hit = w.begin_object(1);
        w.be_f64(2.5); // fHit.fE
        w.end_object(hit);
        w.end_object(event);
    }
    fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
        vec![my_hit(), my_event()]
    }
}

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oxiroot_collection_streamers_{name}.root"))
}

/// The infos a file describes for `classes`, in file order.
fn described(file: &FileReader, classes: &[&str]) -> Vec<StreamerInfo> {
    file.streamer_registry()
        .unwrap()
        .infos()
        .iter()
        .filter(|info| classes.contains(&info.class_name.as_str()))
        .cloned()
        .collect()
}

#[test]
fn a_list_read_back_still_describes_its_members() {
    let first = temp("list_a");
    let list = ObjList::new(oxiroot_io_core::ListKind::List)
        .named("events")
        .add(&MyEvent { name: "e1".into() })
        .add(&ObjString::new("note").named("n"));
    FileWriter::create(&first)
        .add(&list)
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let source = FileReader::open(&first).unwrap();
    let expected = described(&source, &["MyHit", "MyEvent"]);
    assert_eq!(expected.len(), 2, "the first file describes both classes");

    // Read the list back and write it on its own: nothing but the file it came
    // from knows MyEvent or MyHit.
    let back = source.get::<ObjList>("events").unwrap();
    let second = temp("list_b");
    FileWriter::create(&second)
        .add(&back)
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let copy = FileReader::open(&second).unwrap();
    assert_eq!(described(&copy, &["MyHit", "MyEvent"]), expected);
    // The member's dependency comes before it, as ROOT orders them.
    let names: Vec<String> = copy
        .streamer_registry()
        .unwrap()
        .infos()
        .iter()
        .map(|info| info.class_name.clone())
        .collect();
    let at = |class: &str| names.iter().position(|n| n == class).unwrap();
    assert!(at("MyHit") < at("MyEvent"), "{names:?}");
    assert_eq!(copy.get::<ObjList>("events").unwrap(), back);

    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
}

#[test]
fn a_map_read_back_still_describes_its_values() {
    let first = temp("map_a");
    let map = ObjMap::new()
        .named("by_run")
        .insert("run7", &MyEvent { name: "e".into() });
    FileWriter::create(&first)
        .add(&map)
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let source = FileReader::open(&first).unwrap();
    let expected = described(&source, &["MyHit", "MyEvent"]);

    let back = source.get::<ObjMap>("by_run").unwrap();
    let second = temp("map_b");
    FileWriter::create(&second)
        .add(&back)
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let copy = FileReader::open(&second).unwrap();
    assert_eq!(described(&copy, &["MyHit", "MyEvent"]), expected);

    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
}

#[test]
fn two_versions_of_a_class_are_both_described() {
    // Objects written by different releases keep their own class versions, so
    // a file may need both descriptions; neither may shadow the other.
    struct Versioned(i32);
    impl WriteRoot for Versioned {
        fn root_class(&self) -> Cow<'static, str> {
            Cow::Borrowed("MyHit")
        }
        fn root_name(&self) -> &str {
            if self.0 == 1 { "old" } else { "new" }
        }
        fn root_title(&self) -> &str {
            ""
        }
        fn write_body(&self, w: &mut WBuffer) {
            let hit = w.begin_object(u16::try_from(self.0).expect("a small test tag"));
            w.be_f64(1.0);
            w.end_object(hit);
        }
        fn streamer_classes(&self) -> Vec<ClassDef<'static>> {
            let mut cls = my_hit();
            cls.version = self.0;
            vec![cls]
        }
    }
    let path = temp("versions");
    FileWriter::create(&path)
        .add(&Versioned(1))
        .add(&Versioned(2))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let versions: Vec<i32> = described(&FileReader::open(&path).unwrap(), &["MyHit"])
        .iter()
        .map(|info| info.class_version)
        .collect();
    assert_eq!(versions, [1, 2]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_list_stored_without_a_name_takes_its_key_name() {
    // ROOT writes a TList with an empty fName under a named key; read back, the
    // list takes the key's name, so it can be written again as it is.
    struct Unnamed(Vec<u8>);
    impl WriteRoot for Unnamed {
        fn root_class(&self) -> Cow<'static, str> {
            Cow::Borrowed("TList")
        }
        fn root_name(&self) -> &str {
            "things"
        }
        fn root_title(&self) -> &str {
            ""
        }
        fn write_body(&self, w: &mut WBuffer) {
            w.bytes(&self.0);
        }
    }
    let path = temp("unnamed");
    let list = ObjList::new(oxiroot_io_core::ListKind::List).add(&ObjString::new("a").named("a"));
    FileWriter::create(&path)
        .add(&Unnamed(list.to_root_bytes()))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let back = FileReader::open(&path)
        .unwrap()
        .get::<ObjList>("things")
        .unwrap();
    assert_eq!(back.name(), "things");
    let again = temp("unnamed_again");
    FileWriter::create(&again)
        .add(&back)
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(again);
}
