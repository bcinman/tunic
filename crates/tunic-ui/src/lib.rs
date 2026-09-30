//! Shared GPUI presentation for Tunic desktop applications.

use gpui::{Context, Div, IntoElement, Render, Stateful, Window, div, prelude::*, rgb};
use tunic_core::{
    Backend, Command, DeviceId, MemoryStore, PresetCatalog, PresetQuery, PresetSummary, ProfileId,
    ProfileName, ProfileSource,
};
use tunic_presets::BundledCatalog;

const SYSTEM_OUTPUT: &str = "system-output";
const FLAT_PROFILE: &str = "flat";

pub struct TunicView {
    model: Model,
}

impl TunicView {
    #[must_use]
    pub fn new() -> Self {
        Self {
            model: Model::new(),
        }
    }
}

impl Default for TunicView {
    fn default() -> Self {
        Self::new()
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
            .child("Device: System Output (audio not connected)")
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
            .when_some(self.model.error.as_ref(), |view, error| {
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
    presets: Vec<PresetSummary>,
    error: Option<String>,
}

impl Model {
    fn new() -> Self {
        let catalog = BundledCatalog;
        let presets = catalog.list(&PresetQuery::default());
        let backend = Backend::new(MemoryStore::default(), catalog)
            .expect("the in-memory store starts with valid state");
        Self {
            backend,
            device: DeviceId::try_new(SYSTEM_OUTPUT).expect("static device ID is valid"),
            presets,
            error: None,
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
            self.error = Some("preset is no longer available".into());
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
        self.error = result.err().map(|error| format!("{error:?}"));
    }

    fn clear_selection(&mut self) {
        self.error = self
            .backend
            .execute(Command::ClearProfile(self.device.clone()))
            .err()
            .map(|error| format!("{error:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::Model;

    #[test]
    fn selecting_profiles_updates_the_authoritative_backend_state() {
        let mut model = Model::new();
        assert_eq!(model.selected_profile_name(), "None");

        model.select_preset(1);
        assert_eq!(model.selected_profile_name(), "Sony MDR-7506");

        model.select_flat();
        assert_eq!(model.selected_profile_name(), "Flat");
        assert_eq!(model.backend.state().profiles.len(), 2);

        model.clear_selection();
        assert_eq!(model.selected_profile_name(), "None");
        assert!(model.error.is_none());
    }
}
