# `tunic-dsp`

Portable, allocation-free stereo equalization. The crate provides validated
peaking, low-shelf, and high-shelf filters, versioned JSON documents, and a
stateful processing graph with zero reported latency.

```rust
use tunic_dsp::{
    Equalizer, EqualizerError, Filter, FrequencyHz, GainDb, PreparedGraph, QualityFactor,
};

fn main() -> Result<(), EqualizerError> {
    let equalizer = Equalizer::with_filter(Filter::peaking(
        FrequencyHz::new(1_000.0)?,
        GainDb::new(6.0)?,
        QualityFactor::new(1.0)?,
    ));

    let mut graph = PreparedGraph::prepare(&equalizer, 48_000.0)?;
    let mut frames = [[0.25, -0.25], [0.5, -0.5]];
    graph.process(&mut frames);
    Ok(())
}
```

`Filter::new` accepts a `FilterKind`; `Filter::peaking`, `low_shelf`, and
`high_shelf` are convenience constructors. Parameter types reject non-finite
values and require positive frequency and Q. Graph preparation additionally
rejects frequencies at or above Nyquist and unstable coefficients.

Use `Equalizer::identity`, `with_filter`, or `with_filters` to build an ordered
cascade. `Equalizer::parse_json` and `to_canonical_json` read and write the
strict versioned document format. `filters` returns the ordered filter slice;
each filter exposes its kind and validated parameters.

## JSON persistence in Tunic

Tunic stores each profile's equalizer as canonical JSON in the SQLite
`profiles.equalizer_json` column. The engine serializes the equalizer when it
creates or saves a profile, then parses and validates it when loading profiles
or restoring state at startup. The document version allows the filter format
to evolve independently of the database schema.

JSON is not involved in real-time processing and is not currently an import or
export format.

`PreparedGraph::process` mutates any number of `[left, right]` frames without
allocating. `PreparedGraph::identity` creates an empty graph. Call `reset` to
clear filter history; `latency_frames` currently returns zero.
