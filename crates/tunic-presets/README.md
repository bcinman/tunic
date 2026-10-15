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

## Representative import batch

Four additional PDFs were downloaded from the official list on 2026-10-15.
All four are imported and reviewed; this was the initial sample, not the full list.
This batch brought the catalog to six presets. No schema extension was necessary for these
recommendations: all explicit adjustments are single-band gain controls.
The full adjustment wording is retained below; it is not exposed as in-app prose.

| JSON under `data/oratory1990/` | Printed date | Bands | Preamp | PDF SHA-256 |
| --- | --- | --- | --- | --- |
| `7hz/salnotes-zero-harman.json` | 02.05.24 | 1–6 | −2.8 dB | `aa7aaba3deba49ab94f1bd124f7eb5de0a2ed963df1cd40b4df11960e9d9f814` |
| `7hz/salnotes-zero-oratory1990.json` | 06.12.22 | 1–5 | −3.3 dB | `331f2fd95482500359e93cfd37b30c8cbf39fafcc67f80305ea008d79a1d9407` |
| `anker/soundcore-life-q30-anc-off-harman.json` | 16.01.21 | 1–10 | −5.1 dB | `cb9d07c1103600cde5dbc5a303cda321014801625b966075c7e6844d5c57b01c` |
| `anker/soundcore-life-q30-anc-on-harman.json` | 16.01.21 | 1–10 | −3.5 dB | `6ea800e15ecdbc3fd1c8b40dbfc35b589a92697f374e11d299d18fa9b1ae3797` |

Original URLs are in each JSON. The wiki calls the second Zero preset
“oratory1990 target”; its PDF names the target “USOUND 1V1 In-Ear Target”, which
the catalog preserves. Blank table rows are omitted, not synthesized as filters.
Band order and IDs are retained, including the USOUND low shelf at band 5.
Q30 variants require the corresponding headphone ANC state; selecting a preset
does not change the headphone's ANC setting.

### Source adjustment wording

Salnotes Zero, Harman:

```text
Adjust gain of band 1 to preference (bass)
Adjust gain of band 2 to preference (warmth)
Adjust gain of band 5 to preference (treble)
Adjust gain of band 6 to preference (airiness)
```

Salnotes Zero, USOUND: no band adjustment recommendations. The PDF specifies
“Pre-gain to avoid clipping: -3,3 dB”, mapped to the base preamp without adding
a named control.

Soundcore Life Q30, ANC off:

```text
Adjust gain of band 2 to preference (Bass)
Adjust gain of band 4 to preference (boxiness/tonality)
Adjust gain of band 7 to preference (treble presence)
Adjust gain of band 10 to preference (airiness)
```

Soundcore Life Q30, ANC on:

```text
Adjust gain of band 1 to preference (Bass)
Adjust gain of band 5 to preference (boxiness/tonality)
Adjust gain of band 8 to preference (treble presence)
Adjust gain of band 10 to preference (airiness)
```

Labels normalize capitalization and slash spacing only. These PDFs prescribe no
adjustment ranges; Tunic's slider limits are not source recommendations. The
published preamps are preserved, not a guarantee against clipping after edits.

### Reproducing the review

Download each JSON's `source_url` with `curl -fL`, changing `dl=0` to `dl=1`,
then record `shasum -a 256` for the downloaded PDF. On macOS, extract text without
installing a PDF dependency:

```sh
swift -e 'import PDFKit; import Foundation; for path in CommandLine.arguments.dropFirst() { print("SOURCE: \(path)"); print(PDFDocument(url: URL(fileURLWithPath: path))!.string!) }' /path/to/presets/*.pdf
```

Compare the text and a visual inspection of each PDF against every JSON band,
preamp, date, target, and control reference. This batch used both checks. Text
reading order interleaves graphs and notes; the full-list importer below rejects
ambiguous tables rather than guessing. Run `mise run check` after transcription.

## Full-list snapshot

The saved wiki (latest update 13.4.2026) contains 1,070 entries linking to 1,069
unique PDFs. All PDFs were downloaded. **1,062 entries are bundled**; eight are
blocked: five use unsupported `LOW_PASS` filters, two print negative Q values,
and one contains two tables without a clear instruction for combining them.
The negative Q values and two-table ambiguity were visually confirmed.

`ISSUES.md` lists all eight blockers and 138 imported entries with review notes:
unmapped instructions, suspicious/dangling references, duplicate labels,
plot/table inconsistencies, and extraction exceptions. This is an automated import
with visual spot checks, not a claim that every PDF was manually reviewed.
Simple, explicit single-band gain
recommendations become controls. Other instructions remain in the manifest;
they are not silently translated to different operations or displayed in-app.
Multi-band, frequency/Q, and conditional adjustment support remains future work.

`oratory1990-manifest.json` accounts for every index row, including shared PDFs,
and preserves index identity, original URLs, PDF hashes, extracted source text,
recommendations, issues, and the output preset IDs/paths. It is review metadata,
not loaded by the application. Table values use Q, never the adjacent bandwidth.
Blank bands are omitted without renumbering later bands; zero-gain bands remain.
Graphic and in-app tables with complete supported filter parameters import just
like parametric tables. Targets use the names printed in the PDFs, while model
and configuration names come from the index. The six earlier preset IDs remain
unchanged. New IDs include a source-URL hash to distinguish otherwise identical
model/variant/target labels.

### Reimporting

No third-party Python packages are needed. Python is pinned in mise; PDF text
extraction uses macOS PDFKit. The importer caches downloads, checks PDF signatures,
uses four concurrent downloads with bounded retries, and keeps source URLs intact.
Run from the repository root:

```sh
mise run import-oratory /tmp/tunic-oratory-index.json .amp/in/oratory-full
# The checked-in manifest can replace the original Reddit JSON on later runs:
mise run import-oratory crates/tunic-presets/oratory1990-manifest.json .amp/in/oratory-full
# Regenerate from already downloaded/extracted sources without network access:
mise run import-oratory crates/tunic-presets/oratory1990-manifest.json .amp/in/oratory-full --offline
mise run check
```

The importer updates catalog JSON, the manifest, and `ISSUES.md`; review the diff.
It does not delete existing presets when sources disappear or become blocked;
the inventory coverage test catches stale/unaccounted catalog entries. Importer
tests exercise decimal commas, Q versus bandwidth, missing bands, invalid values,
malformed recommendations, and legacy table ordering without network access.

`oratory1990-overrides.json` records one visually verified preamp that PDFKit
does not extract (Jabra Evolve 40). Overrides require the exact source PDF hash;
a changed PDF blocks that entry pending review. All source prose is retained so
exception handling can improve without losing the original instructions.
