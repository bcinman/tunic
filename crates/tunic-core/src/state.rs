use crate::{Profile, ProfileId};
use nutype::nutype;

#[nutype(
    validate(not_empty),
    derive(Clone, Debug, Display, Eq, Hash, PartialEq, TryFrom)
)]
pub struct DeviceId(String);

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

#[cfg(test)]
mod tests {
    use super::DeviceId;

    #[test]
    fn device_identifiers_are_not_empty() {
        assert!(DeviceId::try_new("").is_err());
        assert!(DeviceId::try_new("system-output").is_ok());
    }
}
