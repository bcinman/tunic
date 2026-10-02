//! Backend-owned processing profiles and built-in profile presets.
//!
//! A profile owns its editable base chain and named filter controls. The
//! effective chain is derived before crossing into real-time processing.

use std::collections::HashSet;

use crate::{Chain, FilterId, GainDb};
use nutype::nutype;

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct ProfileId(String);

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom, AsRef)
)]
pub struct PresetId(String);

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom, AsRef)
)]
pub struct PresetRevision(String);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct ProfileName(String);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct FilterControlName(String);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProfileRevision(pub u64);

impl ProfileRevision {
    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// A user-facing control targeting one filter in a profile's base chain.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterControl {
    target: FilterId,
    name: FilterControlName,
    gain_adjustment: GainDb,
}

impl FilterControl {
    #[must_use]
    pub fn new(target: FilterId, name: FilterControlName) -> Self {
        Self {
            target,
            name,
            gain_adjustment: GainDb::default(),
        }
    }

    #[must_use]
    pub fn target(&self) -> FilterId {
        self.target
    }

    #[must_use]
    pub fn name(&self) -> &FilterControlName {
        &self.name
    }

    #[must_use]
    pub fn gain_adjustment(&self) -> GainDb {
        self.gain_adjustment
    }
}

/// A saved reusable processing profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    id: ProfileId,
    name: ProfileName,
    base: Chain,
    controls: Vec<FilterControl>,
    revision: ProfileRevision,
    origin: Option<PresetOrigin>,
}

impl Profile {
    pub(crate) fn new(
        id: ProfileId,
        name: ProfileName,
        base: Chain,
        controls: Vec<FilterControl>,
        origin: Option<PresetOrigin>,
    ) -> Result<Self, ProfileError> {
        let profile = Self {
            id,
            name,
            base,
            controls,
            revision: ProfileRevision::default(),
            origin,
        };
        profile.validate()?;
        Ok(profile)
    }

    #[must_use]
    pub fn id(&self) -> &ProfileId {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &ProfileName {
        &self.name
    }

    #[must_use]
    pub fn base(&self) -> &Chain {
        &self.base
    }

    #[must_use]
    pub fn controls(&self) -> &[FilterControl] {
        &self.controls
    }

    #[must_use]
    pub fn control(&self, target: FilterId) -> Option<&FilterControl> {
        self.controls
            .iter()
            .find(|control| control.target == target)
    }

    #[must_use]
    pub fn revision(&self) -> ProfileRevision {
        self.revision
    }

    #[must_use]
    pub fn origin(&self) -> Option<&PresetOrigin> {
        self.origin.as_ref()
    }

    /// Resolves the base and current adjustments into a processing chain.
    #[must_use]
    pub fn effective_chain(&self) -> Chain {
        let mut chain = self.base.clone();
        for control in &self.controls {
            let filter = chain
                .equalizer
                .filters
                .iter_mut()
                .find(|filter| filter.id == control.target)
                .expect("profile invariants require every control target");
            filter.parameters.gain = GainDb::try_new(
                filter.parameters.gain.into_inner() + control.gain_adjustment.into_inner(),
            )
            .expect("profile invariants require a finite effective gain");
        }
        chain
    }

    /// Replaces the base and removes controls whose target no longer exists.
    pub fn replace_base(&mut self, base: Chain) -> Result<Vec<FilterControl>, ProfileError> {
        validate_filter_ids(&base)?;
        validate_effective_gains(&base, &self.controls)?;
        let ids = base
            .equalizer
            .filters
            .iter()
            .map(|filter| filter.id)
            .collect::<HashSet<_>>();
        let mut removed = Vec::new();
        self.controls.retain(|control| {
            if ids.contains(&control.target) {
                true
            } else {
                removed.push(control.clone());
                false
            }
        });
        self.base = base;
        Ok(removed)
    }

    pub fn expose_filter(
        &mut self,
        target: FilterId,
        name: FilterControlName,
    ) -> Result<(), ProfileError> {
        if !self.base.equalizer.filters.iter().any(|f| f.id == target) {
            return Err(ProfileError::FilterNotFound(target));
        }
        if self.control(target).is_some() {
            return Err(ProfileError::ControlAlreadyExists(target));
        }
        if self.controls.iter().any(|control| control.name == name) {
            return Err(ProfileError::ControlNameAlreadyExists(name));
        }
        self.controls.push(FilterControl::new(target, name));
        Ok(())
    }

    pub fn rename_control(
        &mut self,
        target: FilterId,
        name: FilterControlName,
    ) -> Result<(), ProfileError> {
        if self
            .controls
            .iter()
            .any(|control| control.target != target && control.name == name)
        {
            return Err(ProfileError::ControlNameAlreadyExists(name));
        }
        self.controls
            .iter_mut()
            .find(|control| control.target == target)
            .ok_or(ProfileError::ControlNotFound(target))?
            .name = name;
        Ok(())
    }

    pub fn adjust_filter_gain(
        &mut self,
        target: FilterId,
        adjustment: GainDb,
    ) -> Result<(), ProfileError> {
        let base_gain = self
            .base
            .equalizer
            .filters
            .iter()
            .find(|filter| filter.id == target)
            .ok_or(ProfileError::FilterNotFound(target))?
            .parameters
            .gain
            .into_inner();
        if GainDb::try_new(base_gain + adjustment.into_inner()).is_err() {
            return Err(ProfileError::InvalidEffectiveGain(target));
        }
        self.controls
            .iter_mut()
            .find(|control| control.target == target)
            .ok_or(ProfileError::ControlNotFound(target))?
            .gain_adjustment = adjustment;
        Ok(())
    }

    pub fn reset_adjustment(&mut self, target: FilterId) -> Result<(), ProfileError> {
        self.adjust_filter_gain(target, GainDb::default())
    }

    pub fn unexpose_filter(&mut self, target: FilterId) -> Result<(), ProfileError> {
        let position = self
            .controls
            .iter()
            .position(|control| control.target == target)
            .ok_or(ProfileError::ControlNotFound(target))?;
        self.controls.remove(position);
        Ok(())
    }

    pub(crate) fn rename(&mut self, name: ProfileName) {
        self.name = name;
    }

    pub(crate) fn set_revision(&mut self, revision: ProfileRevision) {
        self.revision = revision;
    }

    pub(crate) fn validate(&self) -> Result<(), ProfileError> {
        validate_filter_ids(&self.base)?;
        let mut targets = HashSet::new();
        let mut names = HashSet::new();
        for control in &self.controls {
            if !targets.insert(control.target) {
                return Err(ProfileError::DuplicateControlTarget(control.target));
            }
            if !names.insert(&control.name) {
                return Err(ProfileError::DuplicateControlName(control.name.clone()));
            }
            if !self
                .base
                .equalizer
                .filters
                .iter()
                .any(|filter| filter.id == control.target)
            {
                return Err(ProfileError::FilterNotFound(control.target));
            }
        }
        validate_effective_gains(&self.base, &self.controls)?;
        Ok(())
    }
}

fn validate_effective_gains(base: &Chain, controls: &[FilterControl]) -> Result<(), ProfileError> {
    for control in controls {
        let Some(filter) = base
            .equalizer
            .filters
            .iter()
            .find(|filter| filter.id == control.target)
        else {
            continue;
        };
        if GainDb::try_new(
            filter.parameters.gain.into_inner() + control.gain_adjustment.into_inner(),
        )
        .is_err()
        {
            return Err(ProfileError::InvalidEffectiveGain(control.target));
        }
    }
    Ok(())
}

fn validate_filter_ids(chain: &Chain) -> Result<(), ProfileError> {
    let mut ids = HashSet::new();
    for filter in &chain.equalizer.filters {
        if !ids.insert(filter.id) {
            return Err(ProfileError::DuplicateFilterId(filter.id));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileError {
    DuplicateFilterId(FilterId),
    DuplicateControlTarget(FilterId),
    DuplicateControlName(FilterControlName),
    FilterNotFound(FilterId),
    ControlAlreadyExists(FilterId),
    ControlNotFound(FilterId),
    ControlNameAlreadyExists(FilterControlName),
    InvalidEffectiveGain(FilterId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetSummary {
    pub id: PresetId,
    pub revision: PresetRevision,
    pub brand: String,
    pub model: String,
    pub variant: Option<String>,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attribution {
    pub provider: String,
    pub measurement_source: Option<String>,
    pub source_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresetOrigin {
    pub id: PresetId,
    pub revision: PresetRevision,
    pub attribution: Attribution,
}

/// A catalog-provided starting point for a profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub summary: PresetSummary,
    pub attribution: Attribution,
    pub chain: Chain,
    pub controls: Vec<FilterControl>,
}

#[cfg(test)]
mod tests {
    use super::{
        FilterControl, FilterControlName, PresetId, PresetRevision, Profile, ProfileError,
        ProfileId, ProfileName,
    };
    use crate::{
        Chain, Equalizer, Filter, FilterId, FilterKind, FilterParameters, FrequencyHz, GainDb,
        QualityFactor,
    };

    fn filter(id: u32, frequency: f64, gain: f64) -> Filter {
        Filter {
            id: FilterId::try_new(id).unwrap(),
            parameters: FilterParameters {
                kind: FilterKind::Peaking,
                frequency: FrequencyHz::try_new(frequency).unwrap(),
                gain: GainDb::try_new(gain).unwrap(),
                quality_factor: QualityFactor::try_new(1.0).unwrap(),
            },
        }
    }

    fn chain(filters: Vec<Filter>) -> Chain {
        Chain {
            equalizer: Equalizer {
                preamp: GainDb::default(),
                filters,
            },
        }
    }

    fn profile(filters: Vec<Filter>) -> Profile {
        Profile::new(
            ProfileId::try_new("profile").unwrap(),
            ProfileName::try_new("Profile").unwrap(),
            chain(filters),
            Vec::new(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn identifiers_are_not_empty() {
        assert!(ProfileId::try_new("").is_err());
        assert!(PresetId::try_new("").is_err());
        assert!(PresetRevision::try_new("").is_err());
        assert!(ProfileId::try_new("studio").is_ok());
    }

    #[test]
    fn names_are_trimmed_and_not_blank() {
        assert!(ProfileName::try_new("   ").is_err());
        assert!(FilterControlName::try_new("   ").is_err());
        assert_eq!(
            ProfileName::try_new("  Studio  ").unwrap().into_inner(),
            "Studio"
        );
        assert_eq!(
            FilterControlName::try_new("  Bass  ").unwrap().into_inner(),
            "Bass"
        );
    }

    #[test]
    fn control_lifecycle_keeps_exposure_separate_from_adjustment() {
        let id = FilterId::try_new(1).unwrap();
        let mut profile = profile(vec![filter(1, 100.0, 2.0)]);

        profile
            .expose_filter(id, FilterControlName::try_new("Bass").unwrap())
            .unwrap();
        assert_eq!(
            profile.control(id).unwrap().gain_adjustment(),
            GainDb::default()
        );
        profile
            .rename_control(id, FilterControlName::try_new("Warmth").unwrap())
            .unwrap();
        profile
            .adjust_filter_gain(id, GainDb::try_new(4.0).unwrap())
            .unwrap();
        assert_eq!(
            profile.effective_chain().equalizer.filters[0]
                .parameters
                .gain
                .into_inner(),
            6.0
        );
        profile.reset_adjustment(id).unwrap();
        assert_eq!(profile.controls().len(), 1);
        assert_eq!(profile.effective_chain(), profile.base().clone());
        profile.unexpose_filter(id).unwrap();
        assert!(profile.controls().is_empty());
    }

    #[test]
    fn replacing_base_preserves_controls_by_id_and_removes_missing_targets() {
        let first = FilterId::try_new(1).unwrap();
        let second = FilterId::try_new(2).unwrap();
        let mut profile = profile(vec![filter(1, 100.0, 2.0), filter(2, 200.0, 3.0)]);
        profile
            .expose_filter(first, FilterControlName::try_new("Bass").unwrap())
            .unwrap();
        profile
            .expose_filter(second, FilterControlName::try_new("Body").unwrap())
            .unwrap();
        profile
            .adjust_filter_gain(first, GainDb::try_new(7.0).unwrap())
            .unwrap();

        let removed = profile
            .replace_base(chain(vec![filter(2, 250.0, 4.0), filter(1, 150.0, 5.0)]))
            .unwrap();
        assert!(removed.is_empty());
        assert_eq!(
            profile
                .control(first)
                .unwrap()
                .gain_adjustment()
                .into_inner(),
            7.0
        );
        assert_eq!(
            profile
                .effective_chain()
                .equalizer
                .filters
                .iter()
                .find(|filter| filter.id == second)
                .unwrap()
                .parameters
                .frequency
                .into_inner(),
            250.0
        );

        let removed = profile
            .replace_base(chain(vec![filter(1, 150.0, 5.0)]))
            .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].target(), second);
        assert!(profile.control(first).is_some());
        assert!(profile.control(second).is_none());
    }

    #[test]
    fn construction_rejects_invalid_control_sets() {
        let id = FilterId::try_new(1).unwrap();
        let name = FilterControlName::try_new("Bass").unwrap();
        assert_eq!(
            Profile::new(
                ProfileId::try_new("profile").unwrap(),
                ProfileName::try_new("Profile").unwrap(),
                chain(vec![filter(1, 100.0, 2.0)]),
                vec![
                    FilterControl::new(id, name.clone()),
                    FilterControl::new(id, FilterControlName::try_new("Other").unwrap()),
                ],
                None,
            ),
            Err(ProfileError::DuplicateControlTarget(id))
        );
        assert_eq!(
            Profile::new(
                ProfileId::try_new("profile").unwrap(),
                ProfileName::try_new("Profile").unwrap(),
                chain(vec![filter(1, 100.0, 2.0)]),
                vec![FilterControl::new(FilterId::try_new(2).unwrap(), name)],
                None,
            ),
            Err(ProfileError::FilterNotFound(FilterId::try_new(2).unwrap()))
        );
    }
}
