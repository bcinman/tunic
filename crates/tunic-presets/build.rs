#[path = "src/format.rs"]
mod format;

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn discover(directory: &Path, files: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in std::fs::read_dir(directory).expect("read catalog directory") {
        let path = entry.expect("read catalog entry").path();
        if path.is_dir() {
            discover(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src/format.rs");
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut files = Vec::new();
    discover(&root.join("data"), &mut files);
    let mut presets = files
        .into_iter()
        .map(|path| {
            let json = std::fs::read_to_string(&path).expect("read preset");
            let preset =
                format::decode(&json).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            (path, preset)
        })
        .collect::<Vec<_>>();
    presets.sort_by(|(_, a), (_, b)| {
        let a = &a.summary;
        let b = &b.summary;
        (&a.brand, &a.model, &a.variant, &a.target, a.id.as_ref()).cmp(&(
            &b.brand,
            &b.model,
            &b.variant,
            &b.target,
            b.id.as_ref(),
        ))
    });
    let mut ids = BTreeMap::new();
    let mut brands: BTreeMap<&str, BTreeMap<&str, Vec<usize>>> = BTreeMap::new();
    let mut output = String::from("static ENTRIES: &[Entry] = &[\n");
    for (index, (path, preset)) in presets.iter().enumerate() {
        let s = &preset.summary;
        assert!(
            ids.insert(s.id.as_ref(), index).is_none(),
            "duplicate preset id {}",
            s.id
        );
        brands
            .entry(&s.brand)
            .or_default()
            .entry(&s.model)
            .or_default()
            .push(index);
        writeln!(output,
            "Entry {{ id: {:?}, revision: {:?}, brand: {:?}, model: {:?}, variant: {:?}, target: {:?}, json: include_str!({:?}) }},",
            s.id.as_ref(), s.revision.as_ref(), s.brand, s.model, s.variant, s.target, path
        ).unwrap();
    }
    output.push_str("];\nstatic IDS: &[(&str, usize)] = &[\n");
    for (id, index) in ids {
        writeln!(output, "({id:?}, {index}),").unwrap();
    }
    output.push_str("];\nstatic BRANDS: &[Brand] = &[\n");
    for (brand, models) in brands {
        writeln!(output, "Brand {{ name: {brand:?}, models: &[").unwrap();
        for (model, indices) in models {
            writeln!(
                output,
                "Model {{ name: {model:?}, indices: &{indices:?} }},"
            )
            .unwrap();
        }
        output.push_str("] },\n");
    }
    output.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("catalog.rs"), output).expect("write catalog index");
}
