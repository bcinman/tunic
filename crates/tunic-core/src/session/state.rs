//! The session's current read model.
//!
//! Each device owns its profiles and remembers its selected profile.

use super::{Profile, ProfileId, SessionError, corrupt};
use nutype::nutype;

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct DeviceId(String);

/// The latest read model exposed by the session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    devices: Vec<DeviceProfiles>,
}

impl State {
    pub fn new(devices: Vec<DeviceProfiles>) -> Result<Self, SessionError> {
        let mut ids = std::collections::HashSet::new();
        let mut profiles = std::collections::HashSet::new();
        for device in &devices {
            if !ids.insert(device.device.clone()) {
                return Err(corrupt("duplicate device".into()));
            }
            for profile in &device.profiles {
                if !profiles.insert(profile.id().clone()) {
                    return Err(corrupt("duplicate profile ID".into()));
                }
            }
        }
        Ok(Self { devices })
    }

    pub fn devices(&self) -> &[DeviceProfiles] {
        &self.devices
    }

    pub fn device(&self, id: &DeviceId) -> Option<&DeviceProfiles> {
        self.devices.iter().find(|device| &device.device == id)
    }

    pub(super) fn device_mut(&mut self, id: &DeviceId) -> Option<&mut DeviceProfiles> {
        self.devices.iter_mut().find(|device| &device.device == id)
    }

    pub(super) fn insert(&mut self, device: DeviceProfiles) {
        self.devices.push(device);
    }

    pub(super) fn fresh_profile_id(&self) -> ProfileId {
        let mut next = self
            .devices
            .iter()
            .map(|device| device.profiles.len())
            .sum::<usize>()
            + 1;
        loop {
            let id = ProfileId::try_new(format!("profile-{next}")).expect("generated ID");
            if self.profile(&id).is_none() {
                return id;
            }
            next += 1;
        }
    }

    #[must_use]
    pub fn profile(&self, id: &ProfileId) -> Option<&Profile> {
        self.devices
            .iter()
            .flat_map(|device| &device.profiles)
            .find(|profile| profile.id() == id)
    }

    #[must_use]
    pub fn selected_profile(&self, device: &DeviceId) -> Option<&Profile> {
        self.device(device).map(DeviceProfiles::selected_profile)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceProfiles {
    device: DeviceId,
    pub(super) profiles: Vec<Profile>,
    pub(super) selected: ProfileId,
}

impl DeviceProfiles {
    pub fn new(
        device: DeviceId,
        profiles: Vec<Profile>,
        selected: ProfileId,
    ) -> Result<Self, SessionError> {
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for profile in &profiles {
            profile
                .validate()
                .map_err(|error| corrupt(format!("{error:?}")))?;
            if !ids.insert(profile.id().clone()) || !names.insert(profile.name().clone()) {
                return Err(corrupt("duplicate profile ID or name".into()));
            }
        }
        if !ids.contains(&selected) {
            return Err(corrupt("missing selected profile".into()));
        }
        Ok(Self {
            device,
            profiles,
            selected,
        })
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device
    }
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }
    pub fn selected_profile_id(&self) -> &ProfileId {
        &self.selected
    }
    pub fn selected_profile(&self) -> &Profile {
        self.profiles
            .iter()
            .find(|profile| profile.id() == &self.selected)
            .expect("validated selection")
    }
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
