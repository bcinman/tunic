/// The ordered processing path applied to an audio stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    pub preamp: GainDb,
    pub filters: Vec<Filter>,
}

/// A single filter in a processing [`Chain`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub kind: FilterKind,
    pub frequency: FrequencyHz,
    pub gain: GainDb,
    pub quality_factor: QualityFactor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct FrequencyHz(pub f64);

#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct GainDb(pub f64);

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct QualityFactor(pub f64);
