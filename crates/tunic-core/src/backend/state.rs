//! The backend's current read model.
//!
//! State contains global reusable profiles and each device's optional profile
//! selection; devices do not own profiles.

use super::{Profile, ProfileId};
use nutype::nutype;

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct DeviceId(String);

/// The latest read model exposed by the backend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub profiles: Vec<Profile>,
    pub selections: Vec<DeviceProfileSelection>,
}

impl State {
    #[must_use]
    pub fn profile(&self, id: &ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id() == id)
    }

    #[must_use]
    pub fn selected_profile(&self, device: &DeviceId) -> Option<&Profile> {
        let profile = &self
            .selections
            .iter()
            .find(|selection| &selection.device == device)?
            .profile;
        self.profile(profile)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceProfileSelection {
    pub device: DeviceId,
    pub profile: ProfileId,
}

#[cfg(test)]
mod tests {
    use super::DeviceId;

    #[test]
    fn device_identifiers_are_not_empty() {
        assert!(DeviceId::try_new("").is_err());
        assert!(DeviceId::try_new("system-output").is_ok());
    }
}
