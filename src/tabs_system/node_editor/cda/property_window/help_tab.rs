//! Help tab: docs URL + embedded description
use crate::cunning_core::cda::CDAAsset;
use bevy_egui::egui::{self, TextEdit, Ui};
use cunning_syntax::LanguageId;

pub fn draw(ui: &mut Ui, asset: &mut CDAAsset) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let w = ui.available_width();
        ui.add_space(8.0);

        // External docs URL
        ui.horizontal(|ui| {
            ui.label("Documentation URL:");
            let mut url = asset.help_url.clone().unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut url)
                        .desired_width(400.0)
                        .hint_text("https://..."),
                )
                .changed()
            {
                asset.help_url = if url.is_empty() { None } else { Some(url) };
            }
            if asset.help_url.is_some() {
                let _ = ui.small_button("🔗 Copy");
            }
        });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        // Embedded Markdown description
        ui.label("Embedded Help (Markdown):");
        ui.add_space(4.0);

        let mut content = asset.help_content.clone().unwrap_or_default();
        let mut layouter = cunning_syntax_egui::layouter(LanguageId::Markdown);
        let response = ui.add(
            TextEdit::multiline(&mut content)
                .desired_width(w)
                .desired_rows(20)
                .font(egui::TextStyle::Monospace)
                .layouter(&mut layouter)
                .hint_text("# Title\n\nDescribe how to use this CDA..."),
        );
        if response.changed() {
            asset.help_content = if content.is_empty() {
                None
            } else {
                Some(content)
            };
        }

        ui.add_space(8.0);
        ui.label("Tip: Markdown is supported and will be rendered in the Inspector.");
    });
}
