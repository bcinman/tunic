use crate::{Command, DurableState, PresetId, ProfileId, ProfileName, State, Store, StoreError};

/// Applies product commands and owns Tunic's authoritative state.
///
/// The backend is deliberately opaque until its command and persistence
/// behavior is implemented.
pub struct Backend {
    store: Box<dyn Store>,
    state: State,
}

impl Backend {
    pub fn execute(&mut self, command: Command) -> Result<State, BackendError> {
        let transition = apply(&self.state, command)?;
        if transition.persist {
            self.store
                .save(&DurableState::from(&transition.state))
                .map_err(BackendError::StoreFailed)?;
        }
        self.state = transition.state;
        Ok(self.state.clone())
    }
}

/// A candidate state and whether it must be persisted before becoming authoritative.
struct Transition {
    state: State,
    persist: bool,
}

fn apply(state: &State, command: Command) -> Result<Transition, BackendError> {
    match command {
        Command::SetBypassed(bypassed) => {
            let mut state = state.clone();
            state.bypassed = bypassed;
            Ok(Transition {
                state,
                persist: false,
            })
        }
        _ => Err(BackendError::CommandNotImplemented),
    }
}

/// A rejected command or failed backend operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    ProfileNotFound(ProfileId),
    PresetNotFound(PresetId),
    ProfileNameAlreadyExists(ProfileName),
    StoreFailed(StoreError),
    CommandNotImplemented,
}

#[cfg(test)]
mod tests {
    use super::{Backend, BackendError};
    use crate::{Command, MemoryStore, ProfileId, State};

    #[test]
    fn execute_commits_an_ephemeral_transition_without_saving() {
        let mut backend = Backend {
            store: Box::new(MemoryStore::default()),
            state: State::default(),
        };

        let state = backend.execute(Command::SetBypassed(true)).unwrap();

        assert!(state.bypassed);
        assert!(backend.state.bypassed);
        assert_eq!(backend.store.load().unwrap(), None);
    }

    #[test]
    fn unimplemented_commands_leave_state_unchanged() {
        let mut backend = Backend {
            store: Box::new(MemoryStore::default()),
            state: State::default(),
        };

        let result = backend.execute(Command::DeleteProfile(
            ProfileId::try_new("profile").unwrap(),
        ));

        assert_eq!(result, Err(BackendError::CommandNotImplemented));
        assert_eq!(backend.state, State::default());
        assert_eq!(backend.store.load().unwrap(), None);
    }
}
