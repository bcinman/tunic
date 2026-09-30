use tunic_core::Chain;
use tunic_macos::{AudioSession, DefaultOutputWatcher};

use crate::audio::{ChangeHandler, Connection, Platform};

#[derive(Default)]
pub struct Apple {
    session: Option<AudioSession>,
    watcher: Option<DefaultOutputWatcher>,
}

impl Platform for Apple {
    type Error = tunic_macos::Error;

    fn watch_default_output(&mut self, notify: ChangeHandler) -> Result<(), Self::Error> {
        self.watcher = Some(DefaultOutputWatcher::start(notify)?);
        Ok(())
    }

    fn refresh_default_output(
        &mut self,
        active_chain: &Chain,
    ) -> Result<Option<Connection>, Self::Error> {
        if let Some(session) = self.session.as_ref() {
            match session.is_current_default_output() {
                Ok(true) => return Ok(None),
                Ok(false) => {}
                Err(error) => {
                    self.session = None;
                    return Err(error);
                }
            }
        }

        self.session = None;
        let (session, controller) = AudioSession::start(active_chain.clone())?;
        let connection = Connection {
            device_name: session.device_name().to_owned(),
            controller,
        };
        self.session = Some(session);
        Ok(Some(connection))
    }
}
