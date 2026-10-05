use tunic_core::{Chain, ChangeHandler, Connection, Platform};
use tunic_macos::{AudioSession, DefaultOutputWatcher};

#[derive(Default)]
pub struct Apple {
    session: Option<AudioSession>,
    watcher: Option<DefaultOutputWatcher>,
}

impl Platform for Apple {
    fn watch_default_output(&mut self, notify: ChangeHandler) -> Result<(), String> {
        self.watcher = Some(DefaultOutputWatcher::start(notify).map_err(|e| e.to_string())?);
        Ok(())
    }

    fn refresh_default_output(
        &mut self,
        active_chain: &Chain,
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
        let (session, controller) =
            AudioSession::start(active_chain.clone()).map_err(|e| e.to_string())?;
        let connection = Connection {
            device_name: session.device_name().to_owned(),
            controller,
        };
        self.session = Some(session);
        Ok(Some(connection))
    }
}
