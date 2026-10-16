use crate::devices::{default_output_id, device_uid};
use crate::{AudioSession, DefaultOutputWatcher};
use tunic_core::{Chain, ChangeHandler, Connection, DeviceId, Platform};

/// Core Audio implementation of the portable session's platform contract.
#[derive(Default)]
pub struct MacosPlatform {
    session: Option<AudioSession>,
    watcher: Option<DefaultOutputWatcher>,
}

impl Platform for MacosPlatform {
    fn watch_default_output(&mut self, notify: ChangeHandler) -> Result<(), String> {
        self.watcher = Some(DefaultOutputWatcher::start(notify).map_err(|e| e.to_string())?);
        Ok(())
    }

    fn refresh_default_output(
        &mut self,
        chain_for_device: &dyn Fn(&DeviceId) -> Chain,
    ) -> Result<Option<Connection>, String> {
        if let Some(session) = self.session.as_ref() {
            match session.is_current_default_output() {
                Ok(true) => return Ok(None),
                Ok(false) => {}
                Err(error) => {
                    self.session = None;
                    return Err(error.to_string());
                }
            }
        }

        self.session = None;
        let output = default_output_id().map_err(|e| e.to_string())?;
        let device_id = DeviceId::try_new(device_uid(output).map_err(|e| e.to_string())?)
            .map_err(|e| format!("invalid output UID: {e:?}"))?;
        let (session, controller) =
            AudioSession::start(output, chain_for_device(&device_id)).map_err(|e| e.to_string())?;
        let connection = Connection {
            device_id,
            device_name: session.device_name().to_owned(),
            controller,
        };
        self.session = Some(session);
        Ok(Some(connection))
    }
}
