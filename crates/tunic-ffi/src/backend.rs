//! Native methods translate to core commands; all profile policy stays in core.

use boltffi::*;
use tunic_core as core;
use tunic_presets::BundledCatalog;

use crate::{Adjustment, Attribution, Chain, ProcessorError};

#[data]
pub struct State {
    pub profiles: Vec<Profile>,
    pub selections: Vec<DeviceProfileSelection>,
}

#[data]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub chain: Chain,
    pub revision: u64,
    pub origin: Option<PresetOrigin>,
    pub adjustments: Vec<Adjustment>,
}

#[data]
pub struct PresetOrigin {
    pub id: String,
    pub revision: String,
    pub attribution: Attribution,
}

#[data]
pub struct DeviceProfileSelection {
    pub device: String,
    pub profile: String,
}

#[error]
#[derive(Debug)]
pub enum BackendError {
    InvalidProfileId,
    InvalidProfileName,
    InvalidPresetId,
    InvalidDeviceId,
    InvalidChain {
        reason: ProcessorError,
    },
    ProfileNotFound {
        id: String,
    },
    ProfileAlreadyExists {
        id: String,
    },
    PresetNotFound {
        id: String,
    },
    DuplicateFilterId {
        id: u32,
    },
    ProfileNameAlreadyExists {
        name: String,
    },
    ProfileRevisionMismatch {
        profile: String,
        expected: u64,
        actual: u64,
    },
    ProfileRevisionExhausted {
        id: String,
    },
    StoreUnavailable {
        message: String,
    },
    StoreCorrupt {
        message: String,
    },
}

/// Non-real-time, single-thread-owned profile state. The native app coordinates
/// returned snapshots with its UI and processor; this object never controls audio.
pub struct Backend {
    inner: core::Backend,
}

#[export(single_threaded)]
impl Backend {
    /// Starts an empty, non-persistent session. Dropping it discards its state.
    pub fn new_in_memory() -> Self {
        Self {
            inner: core::Backend::new(core::MemoryStore::default(), BundledCatalog)
                .expect("empty memory store is valid"),
        }
    }

    pub fn state(&self) -> State {
        self.inner.state().clone().into()
    }

    pub fn create_profile_from_preset(
        &mut self,
        id: String,
        name: String,
        preset_id: String,
    ) -> Result<State, BackendError> {
        self.execute(core::Command::CreateProfile {
            id: profile_id(id)?,
            name: profile_name(name)?,
            source: core::ProfileSource::Preset(
                core::PresetId::try_new(preset_id).map_err(|_| BackendError::InvalidPresetId)?,
            ),
        })
    }

    pub fn create_flat_profile(&mut self, id: String, name: String) -> Result<State, BackendError> {
        self.execute(core::Command::CreateProfile {
            id: profile_id(id)?,
            name: profile_name(name)?,
            source: core::ProfileSource::Flat,
        })
    }

    pub fn copy_profile(
        &mut self,
        id: String,
        name: String,
        source_id: String,
    ) -> Result<State, BackendError> {
        self.execute(core::Command::CreateProfile {
            id: profile_id(id)?,
            name: profile_name(name)?,
            source: core::ProfileSource::Copy(profile_id(source_id)?),
        })
    }

    pub fn rename_profile(&mut self, id: String, name: String) -> Result<State, BackendError> {
        self.execute(core::Command::RenameProfile {
            profile: profile_id(id)?,
            name: profile_name(name)?,
        })
    }

    pub fn delete_profile(&mut self, id: String) -> Result<State, BackendError> {
        self.execute(core::Command::DeleteProfile(profile_id(id)?))
    }

    pub fn select_profile(
        &mut self,
        device_id: String,
        profile_id: String,
    ) -> Result<State, BackendError> {
        self.execute(core::Command::SelectProfile {
            device: core::DeviceId::try_new(device_id)
                .map_err(|_| BackendError::InvalidDeviceId)?,
            profile: self::profile_id(profile_id)?,
        })
    }

    pub fn clear_profile(&mut self, device_id: String) -> Result<State, BackendError> {
        self.execute(core::Command::ClearProfile(
            core::DeviceId::try_new(device_id).map_err(|_| BackendError::InvalidDeviceId)?,
        ))
    }

    pub fn update_profile(
        &mut self,
        id: String,
        chain: Chain,
        expected_revision: u64,
    ) -> Result<State, BackendError> {
        self.execute(core::Command::UpdateProfile {
            profile: profile_id(id)?,
            chain: chain
                .try_into()
                .map_err(|reason| BackendError::InvalidChain { reason })?,
            expected_revision: core::ProfileRevision(expected_revision),
        })
    }
}

impl Backend {
    fn execute(&mut self, command: core::Command) -> Result<State, BackendError> {
        self.inner
            .execute(command)
            .map(Into::into)
            .map_err(Into::into)
    }
}

fn profile_id(id: String) -> Result<core::ProfileId, BackendError> {
    core::ProfileId::try_new(id).map_err(|_| BackendError::InvalidProfileId)
}

fn profile_name(name: String) -> Result<core::ProfileName, BackendError> {
    core::ProfileName::try_new(name).map_err(|_| BackendError::InvalidProfileName)
}

impl From<core::State> for State {
    fn from(state: core::State) -> Self {
        Self {
            profiles: state
                .profiles
                .into_iter()
                .map(|profile| Profile {
                    id: profile.id.into_inner(),
                    name: profile.name.into_inner(),
                    chain: profile.chain.into(),
                    revision: profile.revision.0,
                    origin: profile.origin.map(|origin| PresetOrigin {
                        id: origin.id.into_inner(),
                        revision: origin.revision.into_inner(),
                        attribution: origin.attribution.into(),
                    }),
                    adjustments: profile.adjustments.into_iter().map(Into::into).collect(),
                })
                .collect(),
            selections: state
                .selections
                .into_iter()
                .map(|selection| DeviceProfileSelection {
                    device: selection.device.into_inner(),
                    profile: selection.profile.into_inner(),
                })
                .collect(),
        }
    }
}

impl From<core::BackendError> for BackendError {
    fn from(error: core::BackendError) -> Self {
        match error {
            core::BackendError::ProfileNotFound(id) => Self::ProfileNotFound {
                id: id.into_inner(),
            },
            core::BackendError::ProfileAlreadyExists(id) => Self::ProfileAlreadyExists {
                id: id.into_inner(),
            },
            core::BackendError::PresetNotFound(id) => Self::PresetNotFound {
                id: id.into_inner(),
            },
            core::BackendError::DuplicateFilterId(id) => Self::DuplicateFilterId {
                id: id.into_inner(),
            },
            core::BackendError::ProfileNameAlreadyExists(name) => Self::ProfileNameAlreadyExists {
                name: name.into_inner(),
            },
            core::BackendError::ProfileRevisionMismatch {
                profile,
                expected,
                actual,
            } => Self::ProfileRevisionMismatch {
                profile: profile.into_inner(),
                expected: expected.0,
                actual: actual.0,
            },
            core::BackendError::ProfileRevisionExhausted(id) => Self::ProfileRevisionExhausted {
                id: id.into_inner(),
            },
            core::BackendError::StoreFailed(core::StoreError::Unavailable { message }) => {
                Self::StoreUnavailable { message }
            }
            core::BackendError::StoreFailed(core::StoreError::Corrupt { message }) => {
                Self::StoreCorrupt { message }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESET: &str = "oratory1990/sennheiser/hd650/harman";

    #[test]
    fn methods_return_complete_independent_snapshots() {
        let mut backend = Backend::new_in_memory();
        let initial = backend.state();
        let flat = backend
            .create_flat_profile("flat".into(), " Flat ".into())
            .unwrap();
        assert_eq!(flat.profiles[0].name, "Flat");
        assert!(flat.profiles[0].origin.is_none());
        assert!(flat.profiles[0].chain.filters.is_empty());
        let preset = backend
            .create_profile_from_preset("eq".into(), "EQ".into(), PRESET.into())
            .unwrap();
        assert_eq!(preset.profiles.len(), 2);
        assert_eq!(preset.profiles[1].origin.as_ref().unwrap().id, PRESET);
        assert_eq!(
            preset.profiles[1].origin.as_ref().unwrap().revision,
            "2023-09-09.1"
        );
        assert_eq!(
            preset.profiles[1]
                .origin
                .as_ref()
                .unwrap()
                .attribution
                .provider,
            "oratory1990"
        );
        assert_eq!(preset.profiles[1].adjustments[0].filter, 3);
        let copy = backend
            .copy_profile("copy".into(), "Copy".into(), "eq".into())
            .unwrap();
        assert_eq!(copy.profiles[2].chain.preamp_gain_db, -9.3);
        assert_eq!(copy.profiles[2].origin.as_ref().unwrap().id, PRESET);
        assert_eq!(copy.profiles[2].adjustments.len(), 5);
        let renamed = backend
            .rename_profile("copy".into(), "Desk".into())
            .unwrap();
        assert_eq!(renamed.profiles[2].name, "Desk");
        backend
            .select_profile("speakers".into(), "copy".into())
            .unwrap();
        let selected = backend
            .select_profile("headphones".into(), "eq".into())
            .unwrap();
        assert_eq!(selected.selections[0].device, "speakers");
        assert_eq!(selected.selections[0].profile, "copy");
        let cleared = backend.clear_profile("speakers".into()).unwrap();
        assert_eq!(cleared.selections.len(), 1);
        assert_eq!(cleared.selections[0].device, "headphones");
        let deleted = backend.delete_profile("eq".into()).unwrap();
        assert!(deleted.selections.is_empty());
        assert_eq!(
            deleted
                .profiles
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["flat", "copy"]
        );
        assert!(initial.profiles.is_empty());
        assert_eq!(flat.profiles.len(), 1);
        assert!(Backend::new_in_memory().state().profiles.is_empty());
    }

    #[test]
    fn updates_delegate_adjustment_rules_and_preserve_typed_conflicts() {
        let mut backend = Backend::new_in_memory();
        let state = backend
            .create_profile_from_preset("eq".into(), "EQ".into(), PRESET.into())
            .unwrap();
        let mut chain = state.profiles.into_iter().next().unwrap().chain;
        chain.filters.reverse();
        chain
            .filters
            .iter_mut()
            .find(|f| f.id == 3)
            .unwrap()
            .gain_db = 4.0;
        let updated = backend.update_profile("eq".into(), chain, 0).unwrap();
        assert_eq!(updated.profiles[0].revision, 1);
        assert_eq!(updated.profiles[0].adjustments.len(), 5);
        let mut chain = updated.profiles.into_iter().next().unwrap().chain;
        chain
            .filters
            .iter_mut()
            .find(|f| f.id == 3)
            .unwrap()
            .frequency_hz = 200.0;
        let stale_chain = backend.state().profiles.remove(0).chain;
        assert!(
            matches!(backend.update_profile("eq".into(), stale_chain, 0),
            Err(BackendError::ProfileRevisionMismatch { profile, expected: 0, actual: 1 }) if profile == "eq")
        );
        let updated = backend.update_profile("eq".into(), chain, 1).unwrap();
        assert_eq!(updated.profiles[0].revision, 2);
        assert_eq!(updated.profiles[0].adjustments.len(), 4);
        assert!(
            updated.profiles[0]
                .adjustments
                .iter()
                .all(|a| a.filter != 3)
        );
    }

    #[test]
    fn invalid_inputs_and_failed_saves_do_not_publish_state() {
        let mut backend = Backend::new_in_memory();
        assert!(matches!(
            backend.create_flat_profile("".into(), "Name".into()),
            Err(BackendError::InvalidProfileId)
        ));
        assert!(matches!(
            backend.create_flat_profile("id".into(), " ".into()),
            Err(BackendError::InvalidProfileName)
        ));
        assert!(matches!(
            backend.create_profile_from_preset("id".into(), "Name".into(), "".into()),
            Err(BackendError::InvalidPresetId)
        ));
        assert!(
            matches!(backend.create_profile_from_preset("id".into(), "Name".into(), "missing".into()),
            Err(BackendError::PresetNotFound { id }) if id == "missing")
        );
        backend
            .create_flat_profile("id".into(), "Name".into())
            .unwrap();
        assert!(
            matches!(backend.create_flat_profile("id".into(), "Other".into()), Err(BackendError::ProfileAlreadyExists { id }) if id == "id")
        );
        assert!(
            matches!(backend.create_flat_profile("other".into(), "Name".into()), Err(BackendError::ProfileNameAlreadyExists { name }) if name == "Name")
        );
        assert!(matches!(
            backend.clear_profile("".into()),
            Err(BackendError::InvalidDeviceId)
        ));
        assert!(
            matches!(backend.delete_profile("missing".into()), Err(BackendError::ProfileNotFound { id }) if id == "missing")
        );
        assert!(matches!(
            backend.update_profile(
                "id".into(),
                Chain {
                    preamp_gain_db: f64::NAN,
                    filters: vec![]
                },
                0
            ),
            Err(BackendError::InvalidChain {
                reason: ProcessorError::InvalidPreampGain
            })
        ));
        assert_eq!(backend.state().profiles[0].revision, 0);

        struct FailingStore;
        impl core::Store for FailingStore {
            fn load(&mut self) -> Result<Option<core::State>, core::StoreError> {
                Ok(None)
            }
            fn save(&mut self, _: &core::State) -> Result<(), core::StoreError> {
                Err(core::StoreError::Unavailable {
                    message: "disk full".into(),
                })
            }
        }
        let mut backend = Backend {
            inner: core::Backend::new(FailingStore, BundledCatalog).unwrap(),
        };
        assert!(
            matches!(backend.create_flat_profile("id".into(), "Name".into()),
            Err(BackendError::StoreUnavailable { message }) if message == "disk full")
        );
        assert!(backend.state().profiles.is_empty());
    }
}
