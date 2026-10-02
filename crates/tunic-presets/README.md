# `tunic-presets`

Offline, read-only headphone EQ presets. `BundledCatalog` implements
`tunic_core::PresetCatalog`; its handle owns no decoded catalog or cache.
`brands`, `models`, and `list` query build-generated static indexes. `get` decodes
one embedded JSON payload into validated core values. No runtime filesystem or
network access is involved. Unknown IDs return `None`; failure to decode a
build-validated payload is a programming defect. Updates ship with the library.

## Adding a preset

Add one JSON file under `data/<source>/<brand>/<model>-<variant>-<target>.json`,
omitting unnecessary filename qualifiers. JSON fields, not paths, determine
identity and display names. See the existing files for the format; data and
decoder ship together without schema-version negotiation.

- `summary`: stable ID, revision, brand, model, optional variant, and target.
- `attribution`: provider, optional measurement source, and original source URL.
- `equalizer`: preamp in dB and ordered filters with positive integer IDs,
  optional `control_name`, kind (`peaking`, `low_shelf`, `high_shelf`), frequency
  in Hz, gain in dB, and Q. A control name exposes that filter for tweaking while
  leaving the decoded base filter independent of presentation metadata. Exposed
  controls adjust only gain. Only expose filters the source explicitly identifies
  as adjustable.

Preset IDs must be globally unique and remain unchanged when files are renamed.
Filter IDs and control names are unique within a preset; do not renumber filters
when reordering.
The initial entries use source-date plus transcription revision (`YYYY-MM-DD.1`);
bump the revision whenever payload or attribution changes. It is distinct from
the source PDF's template version.

`build.rs` discovers JSON files recursively, validates them using the same decoder
as runtime, and rejects unknown fields, unsupported enums, invalid
values, duplicate IDs, and dangling adjustment references. Catalog summaries are
ordered by brand, model, variant, target, then ID; filters use exact case-sensitive
brand/model matches. Multiple presets for the same model remain separate entries.

Run `mise run check` after adding or editing data. Tests also compile every
bundled chain at 44.1, 48, 96, and 192 kHz. Actual processor construction still
validates against the application's playback rate.

## Initial sources

The two entries were checked against oratory1990's original PDFs linked from the
[official list](https://www.reddit.com/r/oratory1990/wiki/index/list_of_presets/):

- Sennheiser HD650: PDF dated 09.09.23.
- Sony MDR-7506: PDF dated 26.06.23.

Both target Harman AE/OE 2018. Shelf values are Q factors, not shelf slopes.
Source URLs and attribution are preserved in each JSON file. These are authored
oratory1990 presets, not AutoEq results derived from his measurements.

The source list prohibits unlicensed commercial use. Tunic is currently a
noncommercial project; revisit source permissions if distribution terms change.
