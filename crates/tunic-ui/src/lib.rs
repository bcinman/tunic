//! Shared GPUI presentation for Tunic desktop applications.

use gpui::{Context, Div, IntoElement, Render, Stateful, Window, div, prelude::*, rgb};
use tunic_core::{
    Backend, Chain, Command, Controller, DeviceId, MemoryStore, PresetCatalog, PresetQuery,
    PresetSummary, ProfileId, ProfileName, ProfileSource,
};
use tunic_presets::BundledCatalog;

const SYSTEM_OUTPUT: &str = "system-output";
const FLAT_PROFILE: &str = "flat";

pub struct TunicView {
    model: Model,
}

impl TunicView {
    pub fn new(
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
    ) -> Self {
        Self {
            model: Model::new(device_name, controller, audio_error),
        }
    }

    #[must_use]
    pub fn active_chain(&self) -> Chain {
        self.model.active_chain()
    }

    pub fn replace_audio(
        &mut self,
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
    ) {
        self.model.device_name = device_name;
        self.model.controller = controller;
        self.model.audio_error = audio_error;
    }
}

impl Render for TunicView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.model.selected_profile_name();
        let presets = self.model.presets.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_6()
            .bg(rgb(0x1f2023))
            .text_color(rgb(0xf2f2f2))
            .child(div().text_xl().child("Tunic"))
            .child(format!("Device: {}", self.model.device_name))
            .child(format!("Selected profile: {selected}"))
            .child(
                button("flat", "Use Flat").on_click(cx.listener(|this, _, _, cx| {
                    this.model.select_flat();
                    cx.notify();
                })),
            )
            .children(presets.into_iter().enumerate().map(|(index, preset)| {
                let label = format!("Use {} {} — {}", preset.brand, preset.model, preset.target);
                button(("preset", index), label).on_click(cx.listener(move |this, _, _, cx| {
                    this.model.select_preset(index);
                    cx.notify();
                }))
            }))
            .child(
                button("clear", "Clear selection").on_click(cx.listener(|this, _, _, cx| {
                    this.model.clear_selection();
                    cx.notify();
                })),
            )
            .when_some(self.model.audio_error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_color(rgb(0xff8a8a))
                        .child(format!("Audio unavailable: {error}")),
                )
            })
            .when_some(self.model.action_error.as_ref(), |view, error| {
                view.child(
                    div()
                        .text_color(rgb(0xff8a8a))
                        .child(format!("Error: {error}")),
                )
            })
    }
}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<gpui::SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .bg(rgb(0x35373c))
        .hover(|style| style.bg(rgb(0x484b52)))
        .cursor_pointer()
        .child(label.into())
}

struct Model {
    backend: Backend,
    device: DeviceId,
    device_name: String,
    controller: Option<Controller>,
    presets: Vec<PresetSummary>,
    audio_error: Option<String>,
    action_error: Option<String>,
}

impl Model {
    fn new(
        device_name: String,
        controller: Option<Controller>,
        audio_error: Option<String>,
    ) -> Self {
        let catalog = BundledCatalog;
        let presets = catalog.list(&PresetQuery::default());
        let backend = Backend::new(MemoryStore::default(), catalog)
            .expect("the in-memory store starts with valid state");
        Self {
            backend,
            device: DeviceId::try_new(SYSTEM_OUTPUT).expect("static device ID is valid"),
            device_name,
            controller,
            presets,
            audio_error,
            action_error: None,
        }
    }

    fn selected_profile_name(&self) -> String {
        self.backend
            .state()
            .selected_profile(&self.device)
            .map_or_else(|| "None".into(), |profile| profile.name.to_string())
    }

    fn select_flat(&mut self) {
        self.select(
            ProfileId::try_new(FLAT_PROFILE).expect("static profile ID is valid"),
            ProfileName::try_new("Flat").expect("static profile name is valid"),
            ProfileSource::Flat,
        );
    }

    fn select_preset(&mut self, index: usize) {
        let Some(preset) = self.presets.get(index).cloned() else {
            self.action_error = Some("preset is no longer available".into());
            return;
        };
        let id = ProfileId::try_new(format!("preset-{index}"))
            .expect("generated profile ID is not empty");
        let name = ProfileName::try_new(format!("{} {}", preset.brand, preset.model))
            .expect("catalog brand and model produce a non-empty name");
        self.select(id, name, ProfileSource::Preset(preset.id));
    }

    fn select(&mut self, id: ProfileId, name: ProfileName, source: ProfileSource) {
        let result = if self.backend.state().profile(&id).is_none() {
            self.backend.execute(Command::CreateProfile {
                id: id.clone(),
                name,
                source,
            })
        } else {
            Ok(self.backend.state().clone())
        }
        .and_then(|_| {
            self.backend.execute(Command::SelectProfile {
                device: self.device.clone(),
                profile: id,
            })
        });
        self.action_error = result
            .err()
            .map(|error| format!("{error:?}"))
            .or_else(|| self.publish_selected_chain());
    }

    fn clear_selection(&mut self) {
        self.action_error = self
            .backend
            .execute(Command::ClearProfile(self.device.clone()))
            .err()
            .map(|error| format!("{error:?}"))
            .or_else(|| self.publish_chain(Chain::default()));
    }

    fn active_chain(&self) -> Chain {
        self.backend
            .state()
            .selected_profile(&self.device)
            .map_or_else(Chain::default, |profile| profile.chain.clone())
    }

    fn publish_selected_chain(&self) -> Option<String> {
        self.publish_chain(self.active_chain())
    }

    fn publish_chain(&self, chain: Chain) -> Option<String> {
        self.controller
            .as_ref()
            .and_then(|controller| controller.set_chain(chain).err())
            .map(|error| format!("publish chain: {error:?}"))
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::Model;
    use tunic_core::{AudioFormat, Chain, Processor, SampleRateHz};

    #[test]
    fn selecting_profiles_updates_the_authoritative_backend_state() {
        let mut model = Model::new("System Output".into(), None, None);
        assert_eq!(model.selected_profile_name(), "None");

        model.select_preset(1);
        assert_eq!(model.selected_profile_name(), "Sony MDR-7506");

        model.select_flat();
        assert_eq!(model.selected_profile_name(), "Flat");
        assert_eq!(model.backend.state().profiles.len(), 2);

        model.clear_selection();
        assert_eq!(model.selected_profile_name(), "None");
        assert!(model.action_error.is_none());
    }

    #[test]
    fn selecting_a_preset_publishes_its_chain_to_the_processor() {
        let (mut processor, controller) = Processor::new(
            AudioFormat {
                sample_rate: SampleRateHz::try_new(48_000.0).unwrap(),
                maximum_frame_count: NonZeroUsize::new(512).unwrap(),
            },
            Chain::default(),
            false,
        )
        .unwrap();
        let mut model = Model::new("System Output".into(), Some(controller), None);
        let mut samples = vec![0.25_f32; 512 * 2];

        model.select_preset(1);
        processor.process(&mut samples);

        assert!(samples.iter().any(|sample| *sample != 0.25));
        assert!(model.action_error.is_none());
    }
}
