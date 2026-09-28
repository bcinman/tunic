use nutype::nutype;

/// The ordered processing path applied to an audio stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    pub stages: Vec<Stage>,
}

/// One operation in a processing [`Chain`].
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    Equalizer(Equalizer),
}

/// Gain and frequency shaping applied as one stage of a processing chain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equalizer {
    pub preamp: GainDb,
    pub filters: Vec<Filter>,
}

/// A single filter in an [`Equalizer`].
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

#[nutype(
    validate(finite, greater = 0.0),
    derive(Clone, Copy, Debug, PartialEq, PartialOrd, TryFrom)
)]
pub struct FrequencyHz(f64);

#[nutype(
    validate(finite),
    derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, TryFrom),
    default = 0.0
)]
pub struct GainDb(f64);

#[nutype(
    validate(finite, greater = 0.0),
    derive(Clone, Copy, Debug, PartialEq, PartialOrd, TryFrom)
)]
pub struct QualityFactor(f64);

#[cfg(test)]
mod tests {
    use super::{Chain, Equalizer, FrequencyHz, GainDb, QualityFactor, Stage};

    #[test]
    fn chain_default_is_an_empty_processing_path() {
        assert_eq!(Chain::default().stages, Vec::<Stage>::new());
    }

    #[test]
    fn equalizer_is_a_distinct_processing_stage() {
        let equalizer = Equalizer::default();
        let chain = Chain {
            stages: vec![Stage::Equalizer(equalizer.clone())],
        };

        assert_eq!(chain.stages, vec![Stage::Equalizer(equalizer)]);
    }

    #[test]
    fn audio_parameters_reject_values_outside_their_domains() {
        assert!(FrequencyHz::try_new(0.0).is_err());
        assert!(FrequencyHz::try_new(f64::NAN).is_err());
        assert!(GainDb::try_new(f64::INFINITY).is_err());
        assert!(QualityFactor::try_new(0.0).is_err());
        assert!(QualityFactor::try_new(f64::NEG_INFINITY).is_err());

        assert!(FrequencyHz::try_new(20.0).is_ok());
        assert!(GainDb::try_new(-12.0).is_ok());
        assert!(QualityFactor::try_new(0.7).is_ok());
    }
}
