use crate::{Profile, ProfileId};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeviceId(pub String);

/// The latest read model exposed by the backend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub bypassed: bool,
    pub profiles: Vec<Profile>,
    pub selections: Vec<DeviceProfileSelection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceProfileSelection {
    pub device: DeviceId,
    pub profile: ProfileId,
}
